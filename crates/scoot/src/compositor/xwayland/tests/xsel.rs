//! The X side of the clipboard suites: a selection owner and a selection
//! reader, each on its own thread with its own X connection -- the owner has
//! to answer the window manager's conversion requests while the test thread
//! is busy pumping the compositor, and a reader blocks on a transfer the
//! compositor has to serve.
//!
//! Both speak ICCCM the way `xclip` does, including `INCR` for anything
//! larger than one chunk, so a large payload crosses the bridge the way a
//! real one would: in chunks, each acknowledged by deleting a property.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
    PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::XWAYLAND_PATIENCE;
use crate::compositor::test_support::Harness;

/// The owner's `INCR` chunk: anything larger goes in pieces of this size.
const CHUNK: usize = 64 * 1024;

/// A connection plus the atoms both halves use.
struct Conn {
    conn: RustConnection,
    root: Window,
    visual: u32,
}

impl Conn {
    fn open(display: u32) -> Result<Self, String> {
        let name = super::display_value(display);
        let (conn, screen) = x11rb::connect(Some(name.as_str())).map_err(|e| e.to_string())?;
        let root = conn.setup().roots[screen].root;
        let visual = conn.setup().roots[screen].root_visual;
        Ok(Self { conn, root, visual })
    }

    fn atom(&self, name: &str) -> Result<Atom, String> {
        Ok(self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom)
    }

    /// A small unmapped window that hears property changes on itself.
    fn window(&self) -> Result<Window, String> {
        let window = self.conn.generate_id().map_err(|e| e.to_string())?;
        self.conn
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                -10,
                -10,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                self.visual,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        Ok(window)
    }
}

/// How an [`Owner`] answers.
#[derive(Clone, Debug)]
pub(super) struct OwnerManner {
    /// Answer `TARGETS`. An owner that does not is invisible to the window
    /// manager's bridge -- it never learns the selection's types -- while
    /// still answering a conversion to a type it was never told about.
    pub(super) answers_targets: bool,
    /// Answer a conversion to its type at all. An owner that does not
    /// leaves every paste of it waiting.
    pub(super) answers_data: bool,
    /// More atom names to list in `TARGETS` after the real type, raw bytes
    /// -- an X client may name an atom anything.
    pub(super) extra_targets: Vec<Vec<u8>>,
    /// Take the selection under the *current* owner's window id rather than
    /// a window of its own -- `SetSelectionOwner` accepts any window, and
    /// conversions still come to this client.
    pub(super) borrow_owner_window: bool,
    /// How a conversion to its type is answered.
    pub(super) data: DataManner,
}

/// How an [`Owner`] answers a conversion to its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DataManner {
    /// As ICCCM asks: one property, or `INCR` chunks each waiting for the
    /// requestor's delete.
    Normal,
    /// One property however large, built by appends -- past the X request
    /// size, which bounds only each request.
    SingleProperty,
    /// Announce `INCR`, then never send a chunk.
    StallAfterIncr,
    /// Announce `INCR`, and on the first delete append the whole payload in
    /// chunks without waiting for any further delete.
    AppendWithoutWaiting,
    /// Announce `INCR`, then answer each delete with a single byte, this
    /// long after it -- forever: progress enough never to look idle.
    Trickle(Duration),
}

impl Default for OwnerManner {
    fn default() -> Self {
        Self {
            answers_targets: true,
            answers_data: true,
            extra_targets: Vec::new(),
            borrow_owner_window: false,
            data: DataManner::Normal,
        }
    }
}

/// An X client owning a selection, serving `payload` as `target` on its own
/// thread until dropped.
pub(super) struct Owner {
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    served: Arc<AtomicUsize>,
    sent: Arc<AtomicUsize>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Owner {
    /// Takes `selection` (`CLIPBOARD`, `PRIMARY`) on a fresh connection and
    /// starts serving it. Returns once the X server has made it the owner.
    pub(super) fn start(
        display: u32,
        selection: &'static str,
        target: &'static str,
        payload: Arc<Vec<u8>>,
        manner: OwnerManner,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let served = Arc::new(AtomicUsize::new(0));
        let sent = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = channel();
        let thread = {
            let (stop, lost, served, sent) =
                (stop.clone(), lost.clone(), served.clone(), sent.clone());
            std::thread::spawn(move || {
                let counters = Counters {
                    lost: &lost,
                    served: &served,
                    sent: &sent,
                };
                serve(
                    display, selection, target, &payload, manner, &stop, counters, ready_tx,
                )
            })
        };
        ready_rx
            .recv_timeout(XWAYLAND_PATIENCE)
            .expect("the X owner took its selection");
        Self {
            stop,
            lost,
            served,
            sent,
            thread: Some(thread),
        }
    }

    /// Whether the owner has been told it lost the selection
    /// (`SelectionClear`) -- what happens when someone else takes it.
    pub(super) fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// How many payload bytes the owner has handed out in `INCR` chunks, all
    /// transfers together -- how far a reader has let it get.
    pub(super) fn sent(&self) -> usize {
        self.sent.load(Ordering::Acquire)
    }

    /// How many conversions of its payload the owner has completed.
    pub(super) fn served(&self) -> usize {
        self.served.load(Ordering::Acquire)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let outcome = thread.join().expect("the X owner thread");
            if !std::thread::panicking() {
                outcome.expect("the X owner ran cleanly");
            }
        }
    }
}

/// What an owner reports back to its [`Owner`].
struct Counters<'a> {
    lost: &'a AtomicBool,
    served: &'a AtomicUsize,
    sent: &'a AtomicUsize,
}

/// One `INCR` transfer in flight from the owner: where it goes and how much
/// has been sent.
struct Outgoing {
    property: Atom,
    sent: usize,
    finished: bool,
    /// A trickled byte due at this moment.
    due: Option<Instant>,
}

#[allow(clippy::too_many_arguments)]
fn serve(
    display: u32,
    selection: &str,
    target: &str,
    payload: &[u8],
    manner: OwnerManner,
    stop: &AtomicBool,
    counters: Counters<'_>,
    ready: std::sync::mpsc::Sender<()>,
) -> Result<(), String> {
    let Counters { lost, served, sent } = counters;
    let x = Conn::open(display)?;
    let selection_atom = x.atom(selection)?;
    let target_atom = x.atom(target)?;
    let targets = x.atom("TARGETS")?;
    let incr = x.atom("INCR")?;
    let mut listed = vec![targets, target_atom];
    for name in &manner.extra_targets {
        listed.push(
            x.conn
                .intern_atom(false, name)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .atom,
        );
    }
    let mut window = x.window()?;
    if manner.borrow_owner_window {
        window = x
            .conn
            .get_selection_owner(selection_atom)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .owner;
        if window == x11rb::NONE {
            return Err("there is no owner to borrow a window from".into());
        }
    }
    x.conn
        .set_selection_owner(window, selection_atom, x11rb::CURRENT_TIME)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
    let owner = x
        .conn
        .get_selection_owner(selection_atom)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .owner;
    if owner != window {
        return Err("the X server did not make the owner the owner".into());
    }
    ready.send(()).map_err(|e| e.to_string())?;
    // Keyed by requestor window: ICCCM allows one transfer per requestor
    // property, and the bridge uses a fresh window per transfer.
    let mut outgoing: HashMap<Window, Outgoing> = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        if let DataManner::Trickle(_) = manner.data {
            let now = Instant::now();
            for (requestor, transfer) in &mut outgoing {
                if transfer.due.is_some_and(|due| due <= now) {
                    transfer.due = None;
                    x.conn
                        .change_property8(
                            PropMode::REPLACE,
                            *requestor,
                            transfer.property,
                            target_atom,
                            b"x",
                        )
                        .map_err(|e| e.to_string())?;
                    x.conn.flush().map_err(|e| e.to_string())?;
                    sent.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
        let Some(event) = x.conn.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match event {
            XEvent::SelectionClear(_) => lost.store(true, Ordering::Release),
            XEvent::SelectionRequest(request) => {
                let property = if request.property == x11rb::NONE {
                    request.target
                } else {
                    request.property
                };
                let answered = if request.target == targets {
                    if !manner.answers_targets {
                        continue;
                    }
                    x.conn
                        .change_property32(
                            PropMode::REPLACE,
                            request.requestor,
                            property,
                            AtomEnum::ATOM,
                            &listed,
                        )
                        .map_err(|e| e.to_string())?;
                    true
                } else if request.target == target_atom {
                    if !manner.answers_data {
                        continue;
                    }
                    if manner.data == DataManner::SingleProperty {
                        // Built by appends: each request stays under the X
                        // request size, the property does not.
                        for (index, piece) in payload.chunks(CHUNK).enumerate() {
                            let mode = if index == 0 {
                                PropMode::REPLACE
                            } else {
                                PropMode::APPEND
                            };
                            x.conn
                                .change_property8(
                                    mode,
                                    request.requestor,
                                    property,
                                    target_atom,
                                    piece,
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        served.fetch_add(1, Ordering::AcqRel);
                    } else if payload.len() <= CHUNK && manner.data == DataManner::Normal {
                        x.conn
                            .change_property8(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                target_atom,
                                payload,
                            )
                            .map_err(|e| e.to_string())?;
                        served.fetch_add(1, Ordering::AcqRel);
                    } else {
                        // Watch the requestor's window for the deletes that
                        // pace the transfer, then announce INCR.
                        x.conn
                            .change_window_attributes(
                                request.requestor,
                                &ChangeWindowAttributesAux::new()
                                    .event_mask(EventMask::PROPERTY_CHANGE),
                            )
                            .map_err(|e| e.to_string())?;
                        x.conn
                            .change_property32(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                incr,
                                &[u32::try_from(payload.len()).unwrap_or(u32::MAX)],
                            )
                            .map_err(|e| e.to_string())?;
                        outgoing.insert(
                            request.requestor,
                            Outgoing {
                                property,
                                sent: 0,
                                finished: false,
                                due: None,
                            },
                        );
                    }
                    true
                } else {
                    false
                };
                x.conn
                    .send_event(
                        false,
                        request.requestor,
                        EventMask::NO_EVENT,
                        SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: request.time,
                            requestor: request.requestor,
                            selection: request.selection,
                            target: request.target,
                            property: if answered { property } else { x11rb::NONE },
                        },
                    )
                    .map_err(|e| e.to_string())?;
                x.conn.flush().map_err(|e| e.to_string())?;
            }
            XEvent::PropertyNotify(notify) if notify.state == Property::DELETE => {
                let Some(transfer) = outgoing.get_mut(&notify.window) else {
                    continue;
                };
                if notify.atom != transfer.property {
                    continue;
                }
                if transfer.finished {
                    outgoing.remove(&notify.window);
                    continue;
                }
                match manner.data {
                    DataManner::StallAfterIncr => continue,
                    DataManner::Trickle(interval) => {
                        transfer.due = Some(Instant::now() + interval);
                        continue;
                    }
                    DataManner::AppendWithoutWaiting => {
                        for piece in payload.chunks(CHUNK) {
                            x.conn
                                .change_property8(
                                    PropMode::APPEND,
                                    notify.window,
                                    transfer.property,
                                    target_atom,
                                    piece,
                                )
                                .map_err(|e| e.to_string())?;
                            sent.fetch_add(piece.len(), Ordering::AcqRel);
                        }
                        x.conn.flush().map_err(|e| e.to_string())?;
                        transfer.finished = true;
                        continue;
                    }
                    DataManner::Normal | DataManner::SingleProperty => {}
                }
                let end = (transfer.sent + CHUNK).min(payload.len());
                let chunk = &payload[transfer.sent..end];
                x.conn
                    .change_property8(
                        PropMode::REPLACE,
                        notify.window,
                        transfer.property,
                        target_atom,
                        chunk,
                    )
                    .map_err(|e| e.to_string())?;
                x.conn.flush().map_err(|e| e.to_string())?;
                if chunk.is_empty() {
                    transfer.finished = true;
                    served.fetch_add(1, Ordering::AcqRel);
                }
                sent.fetch_add(chunk.len(), Ordering::AcqRel);
                transfer.sent = end;
            }
            _ => {}
        }
    }
    Ok(())
}

/// What an X reader got.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Read {
    /// The conversion was refused (`SelectionNotify` with no property).
    Refused,
    /// The payload, complete.
    Data(Vec<u8>),
}

/// How an X reader behaves once a transfer is incremental.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pace {
    /// Delete each chunk's property as it arrives: a normal reader.
    Normal,
    /// Never delete the `INCR` property: a stuck reader. The transfer never
    /// advances, and the reader holds its window until dropped.
    Never,
}

/// An X client converting a selection on its own thread.
pub(super) struct Reader {
    result: Receiver<Result<Read, String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    pub(super) fn start(
        display: u32,
        selection: &'static str,
        target: &'static str,
        pace: Pace,
    ) -> Self {
        let (tx, result) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let x = match Conn::open(display) {
                    Ok(x) => x,
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        return;
                    }
                };
                let outcome = read(&x, selection, target, pace, &stop);
                let _ = tx.send(outcome);
                // A stuck reader keeps its connection -- so its window, and
                // so the transfer -- alive until the test is done with it.
                while pace == Pace::Never && !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                drop(x);
            })
        };
        Self {
            result,
            stop,
            thread: Some(thread),
        }
    }

    /// Pumps `fixture` until the reader is done, and hands back what it got.
    pub(super) fn finish<S, A>(&self, fixture: &mut Harness<S, A>) -> Result<Read, String> {
        let deadline = Instant::now() + XWAYLAND_PATIENCE;
        loop {
            if let Ok(outcome) = self.result.try_recv() {
                return outcome;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the X reader; the transfer never finished"
            );
            fixture.settle();
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read(
    x: &Conn,
    selection: &str,
    target: &str,
    pace: Pace,
    stop: &AtomicBool,
) -> Result<Read, String> {
    let selection_atom = x.atom(selection)?;
    let target_atom = x.atom(target)?;
    let property = x.atom("SCOOT_TEST_SELECTION")?;
    let incr = x.atom("INCR")?;
    let window = x.window()?;
    x.conn
        .convert_selection(
            window,
            selection_atom,
            target_atom,
            property,
            x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    x.conn.flush().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + XWAYLAND_PATIENCE;
    let next = |what: &str| -> Result<XEvent, String> {
        loop {
            if stop.load(Ordering::Acquire) {
                return Err(format!("stopped while waiting for {what}"));
            }
            if let Some(event) = x.conn.poll_for_event().map_err(|e| e.to_string())? {
                return Ok(event);
            }
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {what}"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let notify = loop {
        if let XEvent::SelectionNotify(notify) = next("SelectionNotify")? {
            break notify;
        }
    };
    if notify.property == x11rb::NONE {
        return Ok(Read::Refused);
    }
    let first = x
        .conn
        .get_property(false, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    if first.type_ != incr {
        x.conn
            .delete_property(window, property)
            .map_err(|e| e.to_string())?;
        x.conn.flush().map_err(|e| e.to_string())?;
        return Ok(Read::Data(first.value));
    }
    if pace == Pace::Never {
        return Err("stuck on purpose".into());
    }
    // INCR: deleting the announcement starts the transfer; each chunk is a
    // new value, read and deleted; an empty chunk ends it.
    x.conn
        .delete_property(window, property)
        .map_err(|e| e.to_string())?;
    x.conn.flush().map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    loop {
        let XEvent::PropertyNotify(event) = next("an INCR chunk")? else {
            continue;
        };
        if event.atom != property || event.state != Property::NEW_VALUE {
            continue;
        }
        let chunk = x
            .conn
            .get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        x.conn.flush().map_err(|e| e.to_string())?;
        if chunk.value.is_empty() {
            return Ok(Read::Data(data));
        }
        data.extend_from_slice(&chunk.value);
    }
}

/// A payload of `len` bytes that is not all one value, so a truncated or
/// reordered transfer cannot pass for a whole one.
pub(super) fn patterned(len: usize) -> Arc<Vec<u8>> {
    Arc::new((0..len).map(|i| (i % 251) as u8).collect())
}

/// One X client asking for `selection` from `windows` windows of its own at
/// once, never deleting a property. Returns how many conversions were
/// answered with data and how many refused, once all have been answered.
pub(super) fn flood<S, A>(
    fixture: &mut Harness<S, A>,
    display: u32,
    selection: &'static str,
    target: &'static str,
    windows: usize,
) -> (usize, usize) {
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let outcome = (|| -> Result<(usize, usize), String> {
                let x = Conn::open(display)?;
                let selection_atom = x.atom(selection)?;
                let target_atom = x.atom(target)?;
                let property = x.atom("SCOOT_TEST_FLOOD")?;
                for _ in 0..windows {
                    let window = x.window()?;
                    x.conn
                        .convert_selection(
                            window,
                            selection_atom,
                            target_atom,
                            property,
                            x11rb::CURRENT_TIME,
                        )
                        .map_err(|e| e.to_string())?;
                }
                x.conn.flush().map_err(|e| e.to_string())?;
                let (mut answered, mut refused) = (0, 0);
                let deadline = Instant::now() + XWAYLAND_PATIENCE;
                while answered + refused < windows {
                    if Instant::now() > deadline {
                        return Err(format!(
                            "only {} of {windows} conversions were answered",
                            answered + refused
                        ));
                    }
                    match x.conn.poll_for_event().map_err(|e| e.to_string())? {
                        Some(XEvent::SelectionNotify(notify)) if notify.property == x11rb::NONE => {
                            refused += 1;
                        }
                        Some(XEvent::SelectionNotify(_)) => answered += 1,
                        Some(_) => {}
                        None => std::thread::sleep(Duration::from_millis(1)),
                    }
                }
                let _ = tx.send(Ok((answered, refused)));
                // Keep every window, and so every transfer, alive until told.
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok((answered, refused))
            })();
            if let Err(error) = outcome {
                let _ = tx.send(Err(error));
            }
        })
    };
    let deadline = Instant::now() + XWAYLAND_PATIENCE;
    let counts = loop {
        if let Ok(outcome) = rx.try_recv() {
            break outcome.expect("the flooding X client ran");
        }
        assert!(Instant::now() < deadline, "timed out waiting for the flood");
        fixture.settle();
    };
    stop.store(true, Ordering::Release);
    let _ = thread.join();
    counts
}

/// A payload of `chunks` whole `INCR` chunks, each starting with its own
/// index, so the chunks a reader got can be told apart and put in order.
pub(super) fn indexed_chunks(chunks: usize) -> Arc<Vec<u8>> {
    let mut payload = patterned(chunks * CHUNK).to_vec();
    for (index, chunk) in payload.chunks_mut(CHUNK).enumerate() {
        chunk[..8].copy_from_slice(&(index as u64).to_le_bytes());
    }
    Arc::new(payload)
}

/// The `INCR` chunk size an [`Owner`] sends in.
pub(super) const OWNER_CHUNK: usize = CHUNK;
