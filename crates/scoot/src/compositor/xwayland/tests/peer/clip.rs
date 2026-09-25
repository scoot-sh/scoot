//! The peer's clipboard half: the selection devices a Wayland toolkit binds
//! (`wl_data_device` and the primary-selection device, both following
//! keyboard focus) plus a `zwlr_data_control` device, the way a clipboard
//! manager watches the clipboard whatever is focused.
//!
//! A source serves its payload from a writer thread, so a large payload
//! never blocks the peer's own event loop, and counts what it has written:
//! that count is how a test sees how much of a transfer the compositor has
//! actually drained.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1 as primary_manager,
    zwp_primary_selection_device_v1 as primary_device,
    zwp_primary_selection_offer_v1 as primary_offer,
    zwp_primary_selection_source_v1 as primary_source,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as control_device, zwlr_data_control_manager_v1 as control_manager,
    zwlr_data_control_offer_v1 as control_offer, zwlr_data_control_source_v1 as control_source,
};

use super::{Ack, Peer};

/// How long a receive may take before it counts as stuck. Generous: a
/// debug build on a VM moving megabytes through two protocols.
const RECEIVE_PATIENCE: Duration = Duration::from_secs(20);

/// Which selection a step is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor::xwayland::tests) enum Which {
    Clipboard,
    Primary,
}

/// The clipboard steps.
#[derive(Debug)]
pub(in crate::compositor::xwayland::tests) enum ClipStep {
    /// Create the peer's selection devices (all three).
    Bind,
    /// Set the selection through the focus-following device, offering
    /// `payload` as `mime`. The peer's window must hold the keyboard.
    Set {
        which: Which,
        mime: &'static str,
        payload: Arc<Vec<u8>>,
    },
    /// Set the clipboard through the data-control device, as a clipboard
    /// manager (or `wl-copy`) does -- whatever is focused.
    ControlSet {
        mime: &'static str,
        payload: Arc<Vec<u8>>,
    },
    /// The mime types the focus-following device is offered for the current
    /// selection, `None` when there is none.
    Offered(Which),
    /// The same, as the clipboard manager's data-control device sees it.
    ControlOffered(Which),
    /// Receive the current selection (focus-following device) as `mime`,
    /// reading until the other end closes.
    Receive { which: Which, mime: &'static str },
    /// How many bytes the peer's sources have written, all told.
    Written,
    /// Start receiving the current selection as `mime` and keep the read
    /// end, without waiting for the other end to close.
    ReceiveLater { which: Which, mime: &'static str },
    /// For every read `ReceiveLater` started, in order: whether the other
    /// end has closed, and how many bytes have arrived. Reads what is there
    /// without waiting.
    Ended,
    /// Close every read `ReceiveLater` started -- readers going away.
    DropLater,
    /// Read the `index`th read `ReceiveLater` started until the other end
    /// closes, and hand back everything it received.
    DrainLater(usize),
}

/// What the clipboard half holds.
#[derive(Default)]
pub(super) struct Clip {
    pub(super) data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub(super) primary_manager: Option<primary_manager::ZwpPrimarySelectionDeviceManagerV1>,
    pub(super) control_manager: Option<control_manager::ZwlrDataControlManagerV1>,
    data_device: Option<wl_data_device::WlDataDevice>,
    primary_device: Option<primary_device::ZwpPrimarySelectionDeviceV1>,
    control_device: Option<control_device::ZwlrDataControlDeviceV1>,
    data_offers: Vec<(wl_data_offer::WlDataOffer, Vec<String>)>,
    data_selection: Option<wl_data_offer::WlDataOffer>,
    primary_offers: Vec<(primary_offer::ZwpPrimarySelectionOfferV1, Vec<String>)>,
    primary_selection: Option<primary_offer::ZwpPrimarySelectionOfferV1>,
    control_offers: Vec<(control_offer::ZwlrDataControlOfferV1, Vec<String>)>,
    control_selection: Option<control_offer::ZwlrDataControlOfferV1>,
    control_primary: Option<control_offer::ZwlrDataControlOfferV1>,
    /// The payload each live source serves, keyed by nothing: sources are
    /// kept alive here (dropping one does not destroy it, but holding it
    /// keeps the payload reachable for its `send` events).
    data_sources: Vec<(wl_data_source::WlDataSource, Arc<Vec<u8>>)>,
    primary_sources: Vec<(primary_source::ZwpPrimarySelectionSourceV1, Arc<Vec<u8>>)>,
    control_sources: Vec<(control_source::ZwlrDataControlSourceV1, Arc<Vec<u8>>)>,
    written: Arc<AtomicUsize>,
    /// The reads `ReceiveLater` started, what each has received and whether
    /// it has ended.
    later: Vec<Later>,
}

/// One read `ReceiveLater` started.
pub(super) struct Later {
    file: Option<std::fs::File>,
    received: Vec<u8>,
    ended: bool,
}

impl Clip {
    /// Whether the devices are bound -- from then on the peer dispatches
    /// between steps too (see `peer.rs`).
    pub(super) fn bound(&self) -> bool {
        self.data_device.is_some()
    }
}

/// One non-blocking round of the peer's event loop: whatever the compositor
/// has sent is read and dispatched, whatever the peer queued is flushed.
pub(super) fn pump(
    conn: &Connection,
    queue: &mut EventQueue<Peer>,
    peer: &mut Peer,
) -> Result<(), String> {
    conn.flush().map_err(|e| e.to_string())?;
    // `read` does not block: with nothing to read it reports `WouldBlock`.
    if let Some(guard) = queue.prepare_read() {
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    queue.dispatch_pending(peer).map_err(|e| e.to_string())?;
    Ok(())
}

fn mimes_of<O: PartialEq>(offers: &[(O, Vec<String>)], offer: Option<&O>) -> Option<Vec<String>> {
    let offer = offer?;
    offers
        .iter()
        .find(|(known, _)| known == offer)
        .map(|(_, mimes)| mimes.clone())
}

/// Serves `payload` into `fd` from its own thread, counting into `written`.
fn serve(fd: OwnedFd, payload: Arc<Vec<u8>>, written: Arc<AtomicUsize>) {
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(fd);
        for chunk in payload.chunks(64 * 1024) {
            match file.write_all(chunk) {
                Ok(()) => {
                    written.fetch_add(chunk.len(), Ordering::AcqRel);
                }
                // The reader went away: a refused or abandoned transfer.
                Err(_) => return,
            }
        }
    });
}

/// Reads `read_end` until the writer closes it, or gives up.
fn drain(read_end: OwnedFd) -> Result<Vec<u8>, String> {
    rustix::fs::fcntl_setfl(&read_end, rustix::fs::OFlags::NONBLOCK).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(read_end);
    let mut data = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let deadline = Instant::now() + RECEIVE_PATIENCE;
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(data),
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(format!(
                        "the receive stalled after {} bytes; the other end never closed",
                        data.len()
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub(super) fn step(
    peer: &mut Peer,
    queue: &mut EventQueue<Peer>,
    conn: &Connection,
    step: ClipStep,
) -> Result<Ack, String> {
    let qh = queue.handle();
    let roundtrip = |queue: &mut EventQueue<Peer>, peer: &mut Peer| {
        queue.roundtrip(peer).map(|_| ()).map_err(|e| e.to_string())
    };
    match step {
        ClipStep::Bind => {
            let seat = peer.seat.clone().ok_or("no wl_seat")?;
            let data = peer
                .clip
                .data_manager
                .clone()
                .ok_or("no wl_data_device_manager")?;
            let primary = peer
                .clip
                .primary_manager
                .clone()
                .ok_or("no primary-selection manager")?;
            let control = peer
                .clip
                .control_manager
                .clone()
                .ok_or("no wlr data-control manager")?;
            peer.clip.data_device = Some(data.get_data_device(&seat, &qh, ()));
            peer.clip.primary_device = Some(primary.get_device(&seat, &qh, ()));
            peer.clip.control_device = Some(control.get_data_device(&seat, &qh, ()));
            roundtrip(queue, peer)?;
            roundtrip(queue, peer)?;
            Ok(Ack::Done)
        }
        ClipStep::Set {
            which,
            mime,
            payload,
        } => {
            match which {
                Which::Clipboard => {
                    let manager = peer.clip.data_manager.clone().ok_or("no data manager")?;
                    let device = peer.clip.data_device.clone().ok_or("not bound")?;
                    let source = manager.create_data_source(&qh, ());
                    source.offer(mime.to_owned());
                    device.set_selection(Some(&source), 0);
                    peer.clip.data_sources.push((source, payload));
                }
                Which::Primary => {
                    let manager = peer
                        .clip
                        .primary_manager
                        .clone()
                        .ok_or("no primary manager")?;
                    let device = peer.clip.primary_device.clone().ok_or("not bound")?;
                    let source = manager.create_source(&qh, ());
                    source.offer(mime.to_owned());
                    device.set_selection(Some(&source), 0);
                    peer.clip.primary_sources.push((source, payload));
                }
            }
            roundtrip(queue, peer)?;
            Ok(Ack::Done)
        }
        ClipStep::ControlSet { mime, payload } => {
            let manager = peer
                .clip
                .control_manager
                .clone()
                .ok_or("no data-control manager")?;
            let device = peer.clip.control_device.clone().ok_or("not bound")?;
            let source = manager.create_data_source(&qh, ());
            source.offer(mime.to_owned());
            device.set_selection(Some(&source));
            peer.clip.control_sources.push((source, payload));
            roundtrip(queue, peer)?;
            Ok(Ack::Done)
        }
        ClipStep::Offered(which) => {
            roundtrip(queue, peer)?;
            let clip = &peer.clip;
            Ok(Ack::Mimes(match which {
                Which::Clipboard => mimes_of(&clip.data_offers, clip.data_selection.as_ref()),
                Which::Primary => mimes_of(&clip.primary_offers, clip.primary_selection.as_ref()),
            }))
        }
        ClipStep::ControlOffered(which) => {
            roundtrip(queue, peer)?;
            let clip = &peer.clip;
            let offer = match which {
                Which::Clipboard => clip.control_selection.as_ref(),
                Which::Primary => clip.control_primary.as_ref(),
            };
            Ok(Ack::Mimes(mimes_of(&clip.control_offers, offer)))
        }
        ClipStep::Receive { which, mime } => {
            roundtrip(queue, peer)?;
            let (read_end, write_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
                .map_err(|e| e.to_string())?;
            match which {
                Which::Clipboard => peer
                    .clip
                    .data_selection
                    .as_ref()
                    .ok_or("no clipboard selection to receive")?
                    .receive(mime.to_owned(), write_end.as_fd()),
                Which::Primary => peer
                    .clip
                    .primary_selection
                    .as_ref()
                    .ok_or("no primary selection to receive")?
                    .receive(mime.to_owned(), write_end.as_fd()),
            }
            conn.flush().map_err(|e| e.to_string())?;
            // Only the compositor's copy may hold the write end open, or
            // the read below never sees the end.
            drop(write_end);
            Ok(Ack::Bytes(drain(read_end)))
        }
        ClipStep::Written => Ok(Ack::Count(peer.clip.written.load(Ordering::Acquire))),
        ClipStep::ReceiveLater { which, mime } => {
            roundtrip(queue, peer)?;
            let (read_end, write_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
                .map_err(|e| e.to_string())?;
            rustix::fs::fcntl_setfl(&read_end, rustix::fs::OFlags::NONBLOCK)
                .map_err(|e| e.to_string())?;
            match which {
                Which::Clipboard => peer
                    .clip
                    .data_selection
                    .as_ref()
                    .ok_or("no clipboard selection to receive")?
                    .receive(mime.to_owned(), write_end.as_fd()),
                Which::Primary => peer
                    .clip
                    .primary_selection
                    .as_ref()
                    .ok_or("no primary selection to receive")?
                    .receive(mime.to_owned(), write_end.as_fd()),
            }
            conn.flush().map_err(|e| e.to_string())?;
            drop(write_end);
            peer.clip.later.push(Later {
                file: Some(std::fs::File::from(read_end)),
                received: Vec::new(),
                ended: false,
            });
            Ok(Ack::Done)
        }
        ClipStep::Ended => {
            let mut buf = vec![0u8; 64 * 1024];
            for later in peer.clip.later.iter_mut().filter(|later| !later.ended) {
                let Some(file) = later.file.as_mut() else {
                    continue;
                };
                loop {
                    match file.read(&mut buf) {
                        Ok(0) => {
                            later.ended = true;
                            break;
                        }
                        Ok(n) => later.received.extend_from_slice(&buf[..n]),
                        Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                        Err(error) => return Err(error.to_string()),
                    }
                }
            }
            Ok(Ack::Ends(
                peer.clip
                    .later
                    .iter()
                    .map(|later| (later.ended, later.received.len()))
                    .collect(),
            ))
        }
        ClipStep::DropLater => {
            for later in &mut peer.clip.later {
                later.file = None;
            }
            Ok(Ack::Done)
        }
        ClipStep::DrainLater(index) => {
            let later = peer.clip.later.get_mut(index).ok_or("no such read")?;
            let file = later.file.take().ok_or("that read was dropped")?;
            let mut received = std::mem::take(&mut later.received);
            received.extend(drain(OwnedFd::from(file))?);
            later.ended = true;
            Ok(Ack::Bytes(Ok(received)))
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => peer.clip.data_offers.push((id, Vec::new())),
            wl_data_device::Event::Selection { id } => peer.clip.data_selection = id,
            _ => {}
        }
    }

    wayland_client::event_created_child!(Peer, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for Peer {
    fn event(
        peer: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some((_, mimes)) = peer.clip.data_offers.iter_mut().find(|(o, _)| o == offer)
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Peer {
    fn event(
        peer: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_source::Event::Send { fd, .. } = event
            && let Some((_, payload)) = peer.clip.data_sources.iter().find(|(s, _)| s == source)
        {
            serve(fd, payload.clone(), peer.clip.written.clone());
        }
    }
}

impl Dispatch<primary_device::ZwpPrimarySelectionDeviceV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &primary_device::ZwpPrimarySelectionDeviceV1,
        event: primary_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            primary_device::Event::DataOffer { offer } => {
                peer.clip.primary_offers.push((offer, Vec::new()));
            }
            primary_device::Event::Selection { id } => peer.clip.primary_selection = id,
            _ => {}
        }
    }

    wayland_client::event_created_child!(Peer, primary_device::ZwpPrimarySelectionDeviceV1, [
        primary_device::EVT_DATA_OFFER_OPCODE => (primary_offer::ZwpPrimarySelectionOfferV1, ()),
    ]);
}

impl Dispatch<primary_offer::ZwpPrimarySelectionOfferV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        offer: &primary_offer::ZwpPrimarySelectionOfferV1,
        event: primary_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let primary_offer::Event::Offer { mime_type } = event
            && let Some((_, mimes)) = peer
                .clip
                .primary_offers
                .iter_mut()
                .find(|(o, _)| o == offer)
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<primary_source::ZwpPrimarySelectionSourceV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        source: &primary_source::ZwpPrimarySelectionSourceV1,
        event: primary_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let primary_source::Event::Send { fd, .. } = event
            && let Some((_, payload)) = peer.clip.primary_sources.iter().find(|(s, _)| s == source)
        {
            serve(fd, payload.clone(), peer.clip.written.clone());
        }
    }
}

impl Dispatch<control_device::ZwlrDataControlDeviceV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &control_device::ZwlrDataControlDeviceV1,
        event: control_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            control_device::Event::DataOffer { id } => {
                peer.clip.control_offers.push((id, Vec::new()));
            }
            control_device::Event::Selection { id } => peer.clip.control_selection = id,
            control_device::Event::PrimarySelection { id } => peer.clip.control_primary = id,
            _ => {}
        }
    }

    wayland_client::event_created_child!(Peer, control_device::ZwlrDataControlDeviceV1, [
        control_device::EVT_DATA_OFFER_OPCODE => (control_offer::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<control_offer::ZwlrDataControlOfferV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        offer: &control_offer::ZwlrDataControlOfferV1,
        event: control_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let control_offer::Event::Offer { mime_type } = event
            && let Some((_, mimes)) = peer
                .clip
                .control_offers
                .iter_mut()
                .find(|(o, _)| o == offer)
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<control_source::ZwlrDataControlSourceV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        source: &control_source::ZwlrDataControlSourceV1,
        event: control_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let control_source::Event::Send { fd, .. } = event
            && let Some((_, payload)) = peer.clip.control_sources.iter().find(|(s, _)| s == source)
        {
            serve(fd, payload.clone(), peer.clip.written.clone());
        }
    }
}

wayland_client::delegate_noop!(Peer: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Peer: ignore primary_manager::ZwpPrimarySelectionDeviceManagerV1);
wayland_client::delegate_noop!(Peer: ignore control_manager::ZwlrDataControlManagerV1);
