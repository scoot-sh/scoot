//! Pins for the received-fd queue cap scoot gets from its wayland-backend
//! fork (`docs/forks.md`, root `Cargo.toml`'s `[patch.crates-io]`), below
//! every line of scoot's own code.
//!
//! wayland-backend keeps the fds that arrive with a client's bytes in a
//! per-connection queue, and a request takes them out only if its signature
//! has an fd argument. Released 0.3.17 never bounded that queue, so a client
//! attaching fds to fd-less requests could park any number of them in this
//! process: review of PR #236 measured one idle client taking scoot from 18
//! to 999 fds, every newcomer and `scootctl` shed, and the client never
//! killed. The fork disconnects a client that leaves more than a cap
//! unclaimed: one eighth of the soft `RLIMIT_NOFILE` read when the client
//! is created, clamped to 128..=1024 (`fd_pressure::backend_queued_fds`
//! restates it; 1024 at the limit scoot raises to, see `nofile.rs`).
//!
//! These fail if the fork is ever dropped by a repin, a `cargo update` or a
//! rebase, and they pin the cap exactly, at whatever limit this process runs
//! with, because `fd_pressure.rs`'s arithmetic adds it (and the most one
//! read can add on top, [`BACKEND_READ_FDS`]) as terms. They go through a
//! bare `Display` and a raw socket, since no `wayland-client` sends fds on
//! fd-less requests; the legitimate shapes that come closest are in
//! `backend_queue_client.rs`.
//!
//! Parked fds are counted by name: each test sends copies of one memfd whose
//! name no other test uses, so the count is exact while other tests in the
//! same `cargo test` process open and close fds of their own. Every test here
//! holds up to ~1100 fds at once, so each takes the flood lock
//! (`dispatch/tests.rs`).

use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::protocol::ProtocolError;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};

use super::BACKEND_READ_FDS;
use crate::compositor::dispatch::tests::hold_flood_lock;
use crate::compositor::fd_pressure::backend_queued_fds;
use crate::compositor::nofile;

/// What the fork's disconnect posts, on `wl_display`.
pub(super) const QUEUE_ERROR: &str = "too many file descriptors queued";

/// `wl_display.error.invalid_method`, the code the fork posts it with.
const INVALID_METHOD: u32 = 1;

/// The cap a client created now gets. Raised first, as a session raises it,
/// so the soft limit cannot move between this read and the client's
/// creation (nothing lowers it, and nothing raises it past the raise).
fn cap() -> usize {
    let soft = nofile::raise().expect("RLIMIT_NOFILE is readable").soft;
    backend_queued_fds(soft) as usize
}

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
/// requests, 28 per message, back to back, one message past the cap. On
/// released wayland-backend 0.3.17 they all stay parked and the client stays
/// connected.
#[test]
fn fds_parked_on_fd_less_requests_disconnect_the_client_and_close() {
    let _flood = hold_flood_lock();
    let cap = cap();
    let mut rig = Rig::new("scoot-fdq-attack");
    rig.park(cap + 28);
    rig.dispatch();
    rig.assert_disconnected_by_the_queue_bound();
    rig.dispatch();
    assert_eq!(rig.parked(), 0, "the parked fds closed with the client");
}

/// The cap is exactly `backend_queued_fds` of this process's soft limit:
/// that many parked fds are kept, with the client still connected, and one
/// more disconnects it. This is what lets `fd_pressure.rs` add it as a term.
#[test]
fn the_cap_is_exactly_the_one_the_reserve_arithmetic_adds() {
    let _flood = hold_flood_lock();
    let cap = cap();
    let mut rig = Rig::new("scoot-fdq-bound");
    rig.park(cap);
    rig.dispatch();
    assert!(
        rig.reason().is_none(),
        "a client parking exactly {cap} fds was disconnected: {:?}",
        rig.reason()
    );
    assert_eq!(rig.parked(), cap, "the server keeps all {cap}");

    rig.sync_carrying(1);
    rig.dispatch();
    rig.assert_disconnected_by_the_queue_bound();
    rig.dispatch();
    assert_eq!(rig.parked(), 0, "the parked fds closed with the client");
}

/// One read adds at most [`BACKEND_READ_FDS`], and the kernel closes the
/// rest of a larger `SCM_RIGHTS` message rather than keeping them for the
/// next read. So the most a connection holds unclaimed, for a moment inside
/// the read that takes it past the cap (the next check disconnects it), is
/// the cap plus one read. Pinned from below the cap, where the read's fds
/// are kept and can be counted: a 32-fd message on top of `cap - 30` parked
/// leaves the client at the cap or under it, never past it.
#[test]
fn one_read_adds_at_most_one_receive_buffer_of_fds() {
    let _flood = hold_flood_lock();
    let cap = cap();
    let read = BACKEND_READ_FDS as usize;
    let mut rig = Rig::new("scoot-fdq-read");
    rig.park(cap - read);
    rig.dispatch();
    assert_eq!(rig.parked(), cap - read);

    rig.sync_carrying_unchecked(read + 2);
    rig.dispatch();
    assert!(
        rig.reason().is_none(),
        "one read took more than {read} fds: {:?}",
        rig.reason()
    );
    let taken = rig.parked() - (cap - read);
    assert!(
        (28..=read).contains(&taken),
        "one read took {taken} of {} fds, not 28 to {read}",
        read + 2
    );
}
