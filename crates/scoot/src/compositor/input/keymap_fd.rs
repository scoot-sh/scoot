//! Pins for `keymap-fd-shared-offset`: every `wl_keyboard.keymap` fd the
//! compositor sends must be readable in full with plain `read()`, no matter
//! how many clients already read theirs.
//!
//! Smithay's `KeymapFile` wraps one sealed memfd (seeks to 0 once, at
//! creation, in the pinned fork) and hands every client's `wl_keyboard` the
//! same open file description through `SCM_RIGHTS` -- offset included. The
//! first `read()`-based client to read moves the shared offset to the end;
//! every later client reads zero bytes and compiles an empty keymap (real
//! toolkits `mmap` the fd, which is offset-independent, and never notice).
//! The fix hands each send an independent file description, so these drive
//! real `wayland-client` connections that read with `read()` and no seek --
//! the shape a launcher or greeter client takes -- and assert on the bytes.
//!
//! Like the other live-`State` suites this needs a writable
//! `$XDG_RUNTIME_DIR` for the listening socket nothing connects to.

use std::io::Read as _;
use std::os::fd::{AsRawFd as _, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use smithay::input::keyboard::XkbConfig;
use smithay::reexports::wayland_server::backend::protocol::ProtocolError;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

use super::super::decorations::Appearance;
use super::super::dispatch::tests::hold_flood_lock;
use super::super::fd_pressure::backend_queued_fds;
use super::super::nofile;
use super::super::test_support::{Harness, wait_for};
use super::*;

type Fixture = Harness<Step, Ack>;

/// What a test tells its probe client to do. Keymap reads are not steps:
/// every `wl_keyboard.keymap` event is read the moment it arrives and
/// reported over a side channel, so a step can never miss one or see one
/// twice.
#[derive(Debug)]
enum Step {
    /// Pump the connection until `n` keymap events have been read on this
    /// client.
    DrainKeymaps(u64),
    /// Create a virtual keyboard on the seat (the virtual-keyboard edge:
    /// such a client still gets a full seat keymap on its `wl_keyboard`).
    CreateVirtualKeyboard,
    /// Upload a fresh `us` keymap on this client's virtual keyboard, the
    /// way wayvnc does.
    UploadKeymap,
}

#[derive(Debug)]
enum Ack {
    Done,
}

/// One `wl_keyboard.keymap` event as a `read()`-based client saw it: the
/// announced size against the bytes `read()` actually returned, plus the
/// bytes themselves so the test can prove they compile.
struct KeymapRead {
    who: usize,
    format: u32,
    size: u32,
    bytes: Vec<u8>,
}

struct ProbeClient {
    who: usize,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    virtual_keyboard: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    keymaps_read: u64,
    keymaps: Sender<KeymapRead>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for ProbeClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(9), qh, ())),
            "zwp_virtual_keyboard_manager_v1" => {
                client.manager = Some(registry.bind(name, version.min(1), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ProbeClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        if capabilities.contains(wl_seat::Capability::Keyboard) && client.keyboard.is_none() {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for ProbeClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wl_keyboard::Event::Keymap { format, fd, size } = event else {
            return;
        };
        let format = match format {
            WEnum::Value(format) => format as u32,
            _ => u32::MAX,
        };
        // Plain `read()`, deliberately no seek and no `mmap`: this is the
        // shape that saw an empty keymap while the shared offset was at the
        // end. `read_to_end` from the fd's current offset is the whole point.
        let mut bytes = Vec::new();
        std::fs::File::from(fd)
            .read_to_end(&mut bytes)
            .expect("the keymap fd should be readable");
        client.keymaps_read += 1;
        client
            .keymaps
            .send(KeymapRead {
                who: client.who,
                format,
                size,
                bytes,
            })
            .expect("the test is still collecting keymaps");
    }
}

impl Dispatch<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, ()> for ProbeClient {
    fn event(
        _: &mut Self,
        _: &zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        _: zwp_virtual_keyboard_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1, ()> for ProbeClient {
    fn event(
        _: &mut Self,
        _: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
        _: zwp_virtual_keyboard_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The client half: bind the seat (and the virtual-keyboard manager when
/// the session advertises it), take a keyboard, then run steps.
fn run_client(
    who: usize,
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
    keymaps: Sender<KeymapRead>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = ProbeClient {
        who,
        seat: None,
        keyboard: None,
        manager: None,
        virtual_keyboard: None,
        keymaps_read: 0,
        keymaps,
    };
    conn.display().get_registry(&qh, ());
    // Twice: the first round trip binds the seat, the second delivers its
    // capabilities -- and the `wl_keyboard` is only created from those.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    if client.seat.is_none() {
        return Err("no wl_seat was advertised".to_owned());
    }
    while let Ok(step) = steps.recv() {
        match step {
            Step::DrainKeymaps(n) => {
                wait_for(&mut queue, &mut client, "the keymap", |seen| {
                    (seen.keymaps_read >= n).then_some(())
                })?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::CreateVirtualKeyboard => {
                let manager = client
                    .manager
                    .clone()
                    .ok_or("no virtual keyboard manager")?;
                let seat = client.seat.clone().ok_or("no wl_seat")?;
                client.virtual_keyboard = Some(manager.create_virtual_keyboard(&seat, &qh, ()));
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::UploadKeymap => {
                use std::io::Write as _;
                use std::os::fd::AsFd as _;
                let keyboard = client
                    .virtual_keyboard
                    .clone()
                    .ok_or("no virtual keyboard")?;
                let text = test_keymap_text("us");
                let mut file = tempfile::tempfile().map_err(|e| e.to_string())?;
                file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                file.flush().map_err(|e| e.to_string())?;
                keyboard.keymap(1, file.as_fd(), text.len() as u32);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// A keymap compiled in-process, the way wayvnc uploads one.
fn test_keymap_text(layout: &str) -> String {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("a compiled test keymap");
    keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
}

/// Spawn a probe client on the fixture, returning its index and the shared
/// keymap channel it reports on.
fn spawn_probe(fixture: &mut Fixture, keymaps: Sender<KeymapRead>) -> usize {
    // `spawn` takes `FnOnce(UnixStream, Receiver<Step>, Sender<Ack>)`: the
    // index and the side channel move into the script closure.
    let who = fixture.clients_len();
    fixture.spawn(move |stream, steps, acks| run_client(who, stream, steps, acks, keymaps))
}

/// Replace the seat keyboard's keymap, the way a session started with that
/// layout would have compiled it at startup.
fn set_layout(fixture: &mut Fixture, layout: &str) {
    let keyboard = fixture.state.seat.get_keyboard().expect("a seat keyboard");
    keyboard
        .set_xkb_config(
            &mut fixture.state,
            XkbConfig {
                rules: "",
                model: "",
                layout,
                variant: "",
                options: None,
            },
        )
        .expect("xkeyboard-config should compile the layout");
    fixture.settle();
}

/// Collect exactly one fresh keymap read per client, in any order.
fn collect_reads(
    fixture: &mut Fixture,
    keymaps: &Receiver<KeymapRead>,
    who: &[usize],
) -> Vec<KeymapRead> {
    let mut reads = Vec::new();
    while reads.len() < who.len() {
        let read = fixture.wait_for(0, keymaps, "a keymap read");
        assert!(
            who.contains(&read.who),
            "a keymap read arrived from an unexpected client {}",
            read.who
        );
        assert!(
            !reads.iter().any(|seen: &KeymapRead| seen.who == read.who),
            "client {} reported two keymaps for one phase",
            read.who
        );
        reads.push(read);
    }
    reads
}

/// The reads are whole: the announced size arrived, both clients agree, and
/// the bytes compile into a real keymap rather than an empty one.
fn assert_whole(reads: &[KeymapRead]) {
    assert!(!reads.is_empty(), "no keymap reads to check");
    let size = reads[0].size;
    assert!(size > 0, "the compositor announced an empty keymap");
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    for read in reads {
        assert_eq!(
            read.format, 1,
            "client {}: expected the xkb_v1 keymap format",
            read.who
        );
        assert_eq!(
            read.bytes.len() as u32,
            size,
            "client {} read {} of {size} announced bytes with plain read()",
            read.who,
            read.bytes.len(),
        );
        let text = String::from_utf8(read.bytes.clone())
            .unwrap_or_else(|_| panic!("client {}: the keymap is not UTF-8", read.who));
        assert!(
            xkb::Keymap::new_from_string(
                &context,
                text,
                xkb::KEYMAP_FORMAT_TEXT_V1,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
            .is_some(),
            "client {}: the keymap bytes do not compile",
            read.who,
        );
    }
}

/// The bug, end to end: a second `read()`-based client mapping after the
/// first used to get an empty keymap, because every client's fd was the
/// same open file description and the first read moved the shared offset to
/// the end. Fail-first for that shape: before the fix the second client
/// reads zero bytes.
#[test]
fn a_second_read_based_client_gets_the_full_keymap() {
    let mut fixture = Fixture::bare(Appearance::default());
    set_layout(&mut fixture, "us");
    let (keymaps_tx, keymaps_rx) = channel();
    let a = spawn_probe(&mut fixture, keymaps_tx.clone());
    let b = spawn_probe(&mut fixture, keymaps_tx.clone());
    fixture.run_on(a, Step::DrainKeymaps(1));
    fixture.run_on(b, Step::DrainKeymaps(1));
    let reads = collect_reads(&mut fixture, &keymaps_rx, &[a, b]);
    assert_whole(&reads);
}

/// The broadcast edge: `change_keymap` sends to all known keyboards, and
/// with a shared offset only the first client to read after a layout change
/// saw the new map. Both clients re-read here, sequentially, with no seek.
#[test]
fn a_keymap_change_reaches_every_known_keyboard() {
    let mut fixture = Fixture::bare(Appearance::default());
    set_layout(&mut fixture, "us");
    let (keymaps_tx, keymaps_rx) = channel();
    let a = spawn_probe(&mut fixture, keymaps_tx.clone());
    let b = spawn_probe(&mut fixture, keymaps_tx.clone());
    fixture.run_on(a, Step::DrainKeymaps(1));
    fixture.run_on(b, Step::DrainKeymaps(1));
    let before = collect_reads(&mut fixture, &keymaps_rx, &[a, b]);
    assert_whole(&before);

    set_layout(&mut fixture, "de");
    fixture.run_on(a, Step::DrainKeymaps(2));
    fixture.run_on(b, Step::DrainKeymaps(2));
    let after = collect_reads(&mut fixture, &keymaps_rx, &[a, b]);
    assert_whole(&after);
    assert_ne!(
        before[0].bytes, after[0].bytes,
        "a layout change should change the keymap bytes"
    );
}

/// The virtual-keyboard edge: a client holding a virtual keyboard (uploaded
/// keymap and all) still reads the full seat keymap on its `wl_keyboard`,
/// alongside a second such client.
#[test]
fn virtual_keyboard_clients_get_the_full_seat_keymap() {
    let mut fixture = Fixture::bare(Appearance::default());
    super::super::virtual_input::init(&mut fixture.state, true);
    set_layout(&mut fixture, "us");
    let (keymaps_tx, keymaps_rx) = channel();
    let a = spawn_probe(&mut fixture, keymaps_tx.clone());
    let b = spawn_probe(&mut fixture, keymaps_tx.clone());
    fixture.run_on(a, Step::DrainKeymaps(1));
    fixture.run_on(b, Step::DrainKeymaps(1));
    assert_whole(&collect_reads(&mut fixture, &keymaps_rx, &[a, b]));

    fixture.run_on(a, Step::CreateVirtualKeyboard);
    fixture.run_on(a, Step::UploadKeymap);
    fixture.run_on(b, Step::CreateVirtualKeyboard);
    fixture.run_on(b, Step::UploadKeymap);

    set_layout(&mut fixture, "de");
    fixture.run_on(a, Step::DrainKeymaps(2));
    fixture.run_on(b, Step::DrainKeymaps(2));
    assert_whole(&collect_reads(&mut fixture, &keymaps_rx, &[a, b]));
}

// ---------------------------------------------------------------------------
// The `SCM_RIGHTS` full-fd-table edge
// ---------------------------------------------------------------------------
//
// The fix sends an independent file description per keymap, which is one
// more fd in flight per client per send. The wayland-backend fork
// (`dev/forks.md`, the root `Cargo.toml`'s `[patch.crates-io]`) disconnects
// a client that parks more than a cap of received fds unclaimed by any
// request; this pins that the guard still fires with keymap traffic
// flowing, and that a well-behaved client keeps getting whole keymaps
// while a flooder is mid-attack.

/// What the fork's disconnect posts, on `wl_display`.
const QUEUE_ERROR: &str = "too many file descriptors queued";

/// `wl_display.error.invalid_method`, the code the fork posts it with.
const INVALID_METHOD: u32 = 1;

/// Records how the backend ended a connection.
#[derive(Default)]
struct Watch(std::sync::Mutex<Option<Result<ProtocolError, String>>>);

impl ClientData for Watch {
    fn disconnected(&self, _: ClientId, reason: DisconnectReason) {
        let ended = match reason {
            DisconnectReason::ProtocolError(error) => Ok(error),
            other => Err(format!("{other:?}")),
        };
        *self.0.lock().expect("the watch lock") = Some(ended);
    }
}

/// The cap a client created now gets, mirroring the backend-queue suite:
/// raised first, as a session raises it, so the soft limit cannot move
/// between this read and the client's creation.
fn flood_cap() -> usize {
    let soft = nofile::raise().expect("RLIMIT_NOFILE is readable").soft;
    backend_queued_fds(soft) as usize
}

fn memfd(tag: &str) -> std::os::fd::OwnedFd {
    rustix::fs::memfd_create(tag, rustix::fs::MemfdFlags::CLOEXEC).expect("a memfd")
}

/// One `wl_display.sync` (which has no fd argument) in one `sendmsg` with
/// `count` copies of the memfd attached: at most 28, what a real client
/// puts in one message.
fn sync_carrying(client: &UnixStream, file: RawFd, id: &mut u32, count: usize) {
    assert!((1..=28).contains(&count), "one sendmsg carries 1..=28 fds");
    let words = [1u32, (12 << 16), *id];
    *id += 1;
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
    let fds = vec![file; count];
    let payload = std::mem::size_of_val(fds.as_slice());
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE(payload as u32) } as usize;
    let mut control = vec![0u8; space];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    // SAFETY: a zeroed msghdr is a valid empty one; every pointer set below
    // outlives the sendmsg call, and the one control message written fits in
    // `control` by construction (CMSG_SPACE of exactly its payload).
    let sent = unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = space as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(payload as u32) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(header).cast(), fds.len());
        libc::sendmsg(client.as_raw_fd(), &message, libc::MSG_NOSIGNAL)
    };
    assert_eq!(
        sent,
        bytes.len() as isize,
        "sendmsg: {}",
        std::io::Error::last_os_error()
    );
}

/// Keymaps keep flowing to a well-behaved client while a second connection
/// floods the server with parked fds past the fork's cap: the guard still
/// disconnects the flooder, and the survivor's keymap reads stay whole --
/// before, during, and after a layout change broadcast mid-attack.
#[test]
fn keymaps_stay_whole_while_a_flooder_is_disconnected() {
    let _flood = hold_flood_lock();
    let mut fixture = Fixture::bare(Appearance::default());
    set_layout(&mut fixture, "us");
    let (keymaps_tx, keymaps_rx) = channel();
    let survivor = spawn_probe(&mut fixture, keymaps_tx);
    fixture.run_on(survivor, Step::DrainKeymaps(1));
    assert_whole(&collect_reads(&mut fixture, &keymaps_rx, &[survivor]));

    // A second connection that parks fds on fd-less requests, the attack
    // shape the fork's bound exists for. It never binds anything: parked
    // fds queue per connection, keyed on nothing else.
    let (server_end, flooder) = UnixStream::pair().expect("a socket pair");
    let watch = Arc::new(Watch::default());
    fixture
        .state
        .display_handle
        .insert_client(server_end, watch.clone())
        .expect("an inserted client");
    let file = memfd("scoot-keymap-flood");
    let cap = flood_cap();
    let mut left = cap + 28;
    let mut next_id = 2u32;
    while left > 0 {
        let count = left.min(28);
        sync_carrying(&flooder, file.as_raw_fd(), &mut next_id, count);
        left -= count;
    }

    // A layout change while the flood is queued: the broadcast goes out on
    // fresh descriptions while the guard is about to fire.
    set_layout(&mut fixture, "de");
    fixture.run_on(survivor, Step::DrainKeymaps(2));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if watch.0.lock().expect("the watch lock").is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the flooder was never disconnected by the queue bound"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    match &*watch.0.lock().expect("the watch lock") {
        Some(Ok(error)) => assert!(
            error.message.contains(QUEUE_ERROR)
                && error.code == INVALID_METHOD
                && error.object_interface == "wl_display",
            "the flooder was disconnected, but not by the queue bound: {error:?}"
        ),
        other => panic!("the flooder was not disconnected by the queue bound: {other:?}"),
    }

    assert_whole(&collect_reads(&mut fixture, &keymaps_rx, &[survivor]));
}
