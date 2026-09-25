//! Pins for the received-fd queue bound scoot gets from its wayland-backend
//! fork (`docs/forks.md`, root `Cargo.toml`'s `[patch.crates-io]`), below
//! every line of scoot's own code.
//!
//! wayland-backend keeps the fds that arrive with a client's bytes in a
//! per-connection queue, and a request takes them out only if its signature
//! has an fd argument. Released 0.3.17 never bounded that queue, so a client
//! attaching fds to fd-less requests could park any number of them in this
//! process: review of PR #236 measured one idle client taking scoot from 18
//! to 999 fds, every newcomer and `scootctl` shed, and the client never
//! killed. The fork disconnects a client that leaves more than 128 unclaimed
//! (`docs/backlog/resolved/wayland-backend-fd-queue-done.md`).
//!
//! These tests fail if the fork is ever dropped by a repin, a `cargo update`
//! or a rebase, and they pin the bound exactly, because `fd_pressure.rs`'s
//! reserve arithmetic adds [`BACKEND_QUEUED_FDS`] and [`BACKEND_READ_FDS`]
//! as terms: a bound that moved would make that sum wrong silently.
//!
//! - The attack shape and the exact bound go through a bare `Display` and a
//!   raw socket, since no `wayland-client` sends fds on fd-less requests.
//! - The legitimate shape that comes closest goes through a real [`State`]
//!   and a real `wayland-client`. That is the Rust backend's client, which
//!   is the one that can run ahead: it grows its outgoing buffer without
//!   limit and sends every fd past the last 28 ahead of the bytes that claim
//!   them, 28 at a time with one byte each. (libwayland's client flushes
//!   before its 29th fd, so its fds never run ahead of their requests by more
//!   than one `sendmsg`.)
//!
//! Parked fds are counted by name: each test sends copies of one memfd whose
//! name no other test uses, so the count is exact while other tests in the
//! same `cargo test` process open and close fds of their own.
//!
//! [`State`]: crate::compositor::State

use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::protocol::ProtocolError;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_client::protocol::{wl_callback, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use super::{BACKEND_QUEUED_FDS, BACKEND_READ_FDS};
use crate::compositor::decorations::Appearance;
use crate::compositor::dispatch::tests::hold_flood_lock;
use crate::compositor::test_support::Harness;

/// What the fork's disconnect posts, on `wl_display`.
const QUEUE_ERROR: &str = "too many file descriptors queued";

/// `wl_display.error.invalid_method`, the code the fork posts it with.
const INVALID_METHOD: u32 = 1;

/// How many fds in this process's table are the memfd named `tag`.
fn open_copies(tag: &str) -> usize {
    let needle = format!("/memfd:{tag} ");
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|link| link.to_string_lossy().starts_with(&needle))
        .count()
}

fn memfd(tag: &str) -> OwnedFd {
    rustix::fs::memfd_create(tag, rustix::fs::MemfdFlags::CLOEXEC).expect("a memfd")
}

// ---------------------------------------------------------------------------
// The attack shape, over a raw socket
// ---------------------------------------------------------------------------

/// How the backend ended the connection: the protocol error it posted, or
/// any other reason, described.
type Ended = Result<ProtocolError, String>;

/// Records how the backend ended the connection.
#[derive(Default)]
struct Watch(Mutex<Option<Ended>>);

impl ClientData for Watch {
    fn disconnected(&self, _: ClientId, reason: DisconnectReason) {
        let ended = match reason {
            DisconnectReason::ProtocolError(error) => Ok(error),
            other => Err(format!("{other:?}")),
        };
        *self.0.lock().expect("the watch lock") = Some(ended);
    }
}

/// A bare server with one raw client connected to it, which sends
/// `wl_display.sync` requests carrying copies of one memfd.
struct Rig {
    display: Display<()>,
    watch: Arc<Watch>,
    client: UnixStream,
    file: OwnedFd,
    tag: &'static str,
    next_id: u32,
}

impl Rig {
    fn new(tag: &'static str) -> Self {
        let display: Display<()> = Display::new().expect("a display");
        let (server, client) = UnixStream::pair().expect("a socket pair");
        let watch = Arc::new(Watch::default());
        display
            .handle()
            .insert_client(server, watch.clone())
            .expect("an inserted client");
        Self {
            display,
            watch,
            client,
            file: memfd(tag),
            tag,
            next_id: 2,
        }
    }

    /// Sends one `wl_display.sync` (which has no fd argument) in one
    /// `sendmsg` with `count` copies of the memfd attached: at most 28, what
    /// a real client puts in one message.
    fn sync_carrying(&mut self, count: usize) {
        assert!((1..=28).contains(&count), "one sendmsg carries 1..=28 fds");
        self.sync_carrying_unchecked(count);
    }

    /// The same with any number of fds, for the one test about what one read
    /// can take.
    fn sync_carrying_unchecked(&mut self, count: usize) {
        let id = self.next_id;
        self.next_id += 1;
        // wl_display (object 1), opcode 0 (`sync`), 12 bytes: one new_id.
        let words = [1u32, (12 << 16), id];
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
        let fds = vec![self.file.as_raw_fd(); count];
        send_with_fds(&self.client, &bytes, &fds);
    }

    /// Sends `total` fds on fd-less requests, 28 per `sendmsg`, back to back.
    fn park(&mut self, total: usize) {
        let mut left = total;
        while left > 0 {
            let count = left.min(28);
            self.sync_carrying(count);
            left -= count;
        }
    }

    fn dispatch(&mut self) {
        self.display.dispatch_clients(&mut ()).expect("a dispatch");
        self.display.flush_clients().expect("a flush");
    }

    /// Fds the server holds for this client: every open copy of the memfd
    /// but the test's own.
    fn parked(&self) -> usize {
        open_copies(self.tag) - 1
    }

    fn reason(&self) -> Option<Ended> {
        self.watch.0.lock().expect("the watch lock").clone()
    }

    /// Asserts the client was disconnected by the fork's queue bound.
    fn assert_disconnected_by_the_queue_bound(&self) {
        match self.reason() {
            Some(Ok(error)) => {
                assert!(
                    error.message.contains(QUEUE_ERROR)
                        && error.code == INVALID_METHOD
                        && error.object_interface == "wl_display",
                    "disconnected, but not by the queue bound: {error:?}"
                );
            }
            other => panic!(
                "a client parking {} fds was not disconnected by the queue bound \
                 (the wayland-backend fork is not in the build?): {other:?}",
                self.parked()
            ),
        }
    }
}

fn send_with_fds(stream: &UnixStream, bytes: &[u8], fds: &[RawFd]) {
    let payload = std::mem::size_of_val(fds);
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
        libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL)
    };
    assert_eq!(
        sent,
        bytes.len() as isize,
        "sendmsg: {}",
        std::io::Error::last_os_error()
    );
}

/// The PR #236 reviewer's probe, in miniature: fds attached to fd-less
/// requests, 28 per message, back to back. On released wayland-backend
/// 0.3.17 all 140 stay parked and the client stays connected.
#[test]
fn fds_parked_on_fd_less_requests_disconnect_the_client_and_close() {
    let mut rig = Rig::new("scoot-fdq-attack");
    rig.park(140);
    rig.dispatch();
    rig.assert_disconnected_by_the_queue_bound();
    rig.dispatch();
    assert_eq!(rig.parked(), 0, "the parked fds closed with the client");
}

/// The bound is exactly [`BACKEND_QUEUED_FDS`]: that many parked fds are
/// kept, with the client still connected, and one more disconnects it. This
/// is what lets `fd_pressure.rs` add the bound as a term.
#[test]
fn the_bound_is_exactly_the_one_the_reserve_arithmetic_adds() {
    let bound = BACKEND_QUEUED_FDS as usize;
    let mut rig = Rig::new("scoot-fdq-bound");
    rig.park(bound);
    rig.dispatch();
    assert!(
        rig.reason().is_none(),
        "a client parking exactly {bound} fds was disconnected: {:?}",
        rig.reason()
    );
    assert_eq!(rig.parked(), bound, "the server keeps all {bound}");

    rig.sync_carrying(1);
    rig.dispatch();
    rig.assert_disconnected_by_the_queue_bound();
    rig.dispatch();
    assert_eq!(rig.parked(), 0, "the parked fds closed with the client");
}

/// One read adds at most [`BACKEND_READ_FDS`], and the kernel closes the
/// rest of a larger `SCM_RIGHTS` message rather than keeping them for the
/// next read. So the most a connection holds unclaimed, for a moment inside
/// the read that takes it past the bound (the next check disconnects it), is
/// the bound plus one read. Pinned from below the bound, where the read's fds
/// are kept and can be counted: a 32-fd message on top of 98 parked leaves
/// the client at the bound or under it, never past it.
#[test]
fn one_read_adds_at_most_one_receive_buffer_of_fds() {
    let bound = BACKEND_QUEUED_FDS as usize;
    let read = BACKEND_READ_FDS as usize;
    let mut rig = Rig::new("scoot-fdq-read");
    rig.park(bound - read);
    rig.dispatch();
    assert_eq!(rig.parked(), bound - read);

    rig.sync_carrying_unchecked(read + 2);
    rig.dispatch();
    assert!(
        rig.reason().is_none(),
        "one read took more than {read} fds: {:?}",
        rig.reason()
    );
    let taken = rig.parked() - (bound - read);
    assert!(
        (28..=read).contains(&taken),
        "one read took {taken} of {} fds, not 28 to {read}",
        read + 2
    );
}

// ---------------------------------------------------------------------------
// The legitimate shape that comes closest, through a real compositor
// ---------------------------------------------------------------------------

/// `count` pools created and destroyed straight away, all queued before one
/// flush, so the Rust client sends their fds ahead of their bytes.
struct Batch {
    count: u32,
}

type Fixture = Harness<Batch, ()>;

#[derive(Default)]
struct TestClient {
    shm: Option<wl_shm::WlShm>,
    synced: bool,
}

/// The largest batch of fd-carrying requests a Rust-backend client can send
/// in one flush and still be served: 140. Its flush sends every fd past the
/// last 28 ahead of their bytes, 28 at a time with one byte each, so 112 are
/// queued (under the bound) when the final write brings the rest and every
/// request claims its fd. This is where a bound set too tight would first
/// cut off a real client, so it is pinned.
#[test]
fn a_rust_client_batching_140_fd_requests_in_one_flush_is_served() {
    let _flood = hold_flood_lock();
    let tag = "scoot-fdq-batch-140";
    let mut fixture = Fixture::bare(Appearance::default());
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, tag));
    fixture.run(Batch { count: 140 });
    assert_eq!(fixture.state.shm_pools.pools_in_flight(), 0);
}

/// One more and the same flush has 140 queued before its final write, past
/// the bound: the client is disconnected by wayland-backend before scoot
/// sees any of its pools. This is the one legitimate-shaped client the bound
/// refuses (a Rust client queueing more than 140 fd-carrying requests
/// between flushes); pinned so that the documented limit
/// (`docs/protocols.md`) cannot drift from the real one.
#[test]
fn a_rust_client_batching_141_fd_requests_in_one_flush_is_disconnected() {
    let _flood = hold_flood_lock();
    let tag = "scoot-fdq-batch-141";
    let mut fixture = Fixture::bare(Appearance::default());
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, tag));
    let error = fixture.run_expecting_disconnect(Batch { count: 141 });
    assert!(
        error.contains(QUEUE_ERROR) && error.contains("on wl_display"),
        "{error}"
    );
    assert_eq!(fixture.state.shm_pools.pools_in_flight(), 0);
}

fn run_client(
    stream: UnixStream,
    steps: Receiver<Batch>,
    acks: Sender<()>,
    tag: &'static str,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let file = memfd(tag);
    rustix::fs::ftruncate(&file, 4096).map_err(|e| e.to_string())?;
    while let Ok(Batch { count }) = steps.recv() {
        for _ in 0..count {
            shm.create_pool(file.as_fd(), 4096, &qh, ()).destroy();
        }
        sync(&conn, &mut queue, &mut client)?;
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A round trip that cannot lose a protocol error to `EPIPE` (the shape and
/// the race `client_fds/tests/shm.rs`'s `sync` documents): a kill landing
/// mid-flush closes the socket under the rest of the write, with the error
/// already waiting in the receive buffer.
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => {
                let _ = queue.dispatch_pending(client);
                if let Some(guard) = conn.prepare_read() {
                    let _ = guard.read();
                }
                let _ = queue.dispatch_pending(client);
                return Err(why(conn, error));
            }
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| why(conn, e))?;
    }
    Ok(())
}

fn why(conn: &Connection, error: impl std::fmt::Display) -> String {
    match conn.protocol_error() {
        Some(error) => format!(
            "protocol error code {} on {}@{}: {}",
            error.code, error.object_interface, error.object_id, error.message
        ),
        None => error.to_string(),
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_shm" {
                client.shm = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        client.synced = true;
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
