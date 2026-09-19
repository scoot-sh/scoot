//! Tests for the Wayland accept loop, against real listening sockets.
//!
//! The error-kind mapping ([`classify`](super::super::ipc::accept::classify))
//! is shared with the IPC loop and covered there; what is under test here is
//! this loop's own effect on a [`ListeningSocket`] backlog, plus the failure
//! it replaces: an `EMFILE` on `accept` with a connection pending, which is
//! what Smithay's source propagated out of the event loop and killed the
//! compositor with (see `super`'s module doc for the chain).
//!
//! The exhaustion scenario runs in a forked child, under the same discipline
//! as `ipc::accept`'s: `RLIMIT_NOFILE` is process-global, and after `fork`
//! only raw `libc` calls plus the real [`drain`](super::drain) path -- which
//! is allocation- and lock-free by construction. No `assert!` in the child:
//! explicit checks that write to stderr and `_exit`.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::reexports::calloop::{EventLoop, PostAction};
use smithay::reexports::wayland_server::{Display, ListeningSocket};

use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;

use super::super::ipc::accept::Spare;
use super::*;

static NEXT_LISTENER: AtomicU64 = AtomicU64::new(0);

/// Serializes this module's tests against each other.
///
/// `cargo test` runs tests on threads in one process, sharing one fd table.
/// The exhaustion test pins its child's `RLIMIT_NOFILE` at the parent's
/// current usage, so a sibling test opening fds between the count and the
/// fork miscalibrates the child: overcount fails fast (an `accept` succeeds
/// where `EMFILE` is asserted), undercount by two or more hangs it (every
/// shed stays `Stuck`, and the EOF read below would block forever -- the
/// non-blocking reads turn that hang into a fast failure, but the failure
/// would still be a false one). Holding this across every test here keeps
/// this module's own fd churn out of that window. Cross-module churn in the
/// same microsecond window stays theoretically possible -- and fails fast,
/// never hangs -- and `cargo nextest` (one process per test) removes it
/// entirely.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Locks [`SERIAL`], recovering from poisoning: a previous holder panicking
/// (a failed test) must not cascade into every sibling failing on the lock
/// instead of on its own assertions. Mutual exclusion still holds for the
/// guard's lifetime either way, which is all this lock is for.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A bound Wayland listening socket with a unique name, and its path.
fn bound() -> (ListeningSocket, PathBuf) {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .expect("tests need a writable $XDG_RUNTIME_DIR, like every harness suite");
    let name = format!(
        "flexwm-wayland-accept-test-{}-{}",
        std::process::id(),
        NEXT_LISTENER.fetch_add(1, Ordering::Relaxed)
    );
    let socket = ListeningSocket::bind(&name).expect("binds a test socket");
    (socket, PathBuf::from(runtime).join(name))
}

fn connect(path: &PathBuf) -> UnixStream {
    // A blocking `connect` on a unix socket returns once the connection is
    // queued in the listener's backlog, so by the time this returns the
    // server end can accept it -- no sleep, no retry.
    UnixStream::connect(path).expect("connects")
}

fn drain_all(socket: &ListeningSocket, spare: &Spare) -> (PostAction, usize) {
    let mut taken = 0;
    let end = drain(socket, spare, &mut |stream| {
        taken += 1;
        drop(stream);
    });
    (end, taken)
}

// --- the loop against a live listener --------------------------------------

#[test]
fn an_idle_listener_ends_cleanly_without_taking_anything() {
    let _serial = serial();
    let (socket, _path) = bound();
    let spare = Spare::new();
    let (end, taken) = drain_all(&socket, &spare);
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(taken, 0, "nothing was pending");
}

#[test]
fn pending_connections_are_served_then_the_loop_ends() {
    let _serial = serial();
    let (socket, path) = bound();
    let _first = connect(&path);
    let _second = connect(&path);
    let spare = Spare::new();
    let (end, taken) = drain_all(&socket, &spare);
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(taken, 2, "every pending connection is taken");
    // The backlog is empty afterwards, which is what clears a level trigger:
    // the next accept says nothing-pending instead of reporting again.
    assert!(
        matches!(socket.accept(), Ok(None)),
        "the backlog is drained"
    );
}

#[test]
fn a_shed_that_finds_no_backlog_re_arms_the_spare() {
    let _serial = serial();
    // Same disarm as the IPC loop's review finding: the outer EMFILE spends
    // the spare, then the inner accept finds the backlog raced away, and
    // without a re-arm the mitigation stays silently disarmed.
    let (socket, _path) = bound(); // idle: nothing pending
    let spare = Spare::new();
    assert!(spare.is_armed(), "setup: the spare starts armed");
    assert!(
        matches!(shed_one(&socket, &spare), ShedOutcome::BacklogEmpty),
        "an idle listener sheds into an empty backlog"
    );
    assert!(
        spare.is_armed(),
        "a BacklogEmpty shed must re-arm the spare it spent"
    );
}

// --- fd exhaustion, in a forked child --------------------------------------

/// What the child exits with.
const CHILD_PASS: libc::c_int = 0;
const CHILD_FAIL: libc::c_int = 1;

fn child_fail(message: &[u8]) -> ! {
    unsafe {
        libc::write(2, message.as_ptr().cast(), message.len());
        libc::_exit(CHILD_FAIL);
    }
}

fn child_errno() -> libc::c_int {
    unsafe { *libc::__errno_location() }
}

/// Roughly how many fds this process holds, so the child can pin its own
/// table at (not below) current use and its fill loop has almost nothing to
/// do. Read in the parent, while opening fds still works.
fn open_fd_count() -> libc::rlim_t {
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd is readable")
        .count() as libc::rlim_t
}

/// Everything the exhausted child does. `listener`/`spare` are the parent's
/// objects, inherited across the fork; `clients` are the raw fds of the
/// connected test clients. Never returns.
fn exhausted_child(
    listener: &ListeningSocket,
    spare: &Spare,
    clients: &[libc::c_int],
    usage: libc::rlim_t,
) -> ! {
    // Pin this process's table where it is. Existing fds are unaffected;
    // only new allocations fail -- which is exactly the state under test.
    let mut current: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut current) } != 0 {
        child_fail(b"child: getrlimit failed\n");
    }
    let capped = libc::rlimit {
        rlim_cur: usage,
        rlim_max: current.rlim_max,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &capped) } != 0 {
        child_fail(b"child: setrlimit failed\n");
    }
    // Fill to the cap with raw opens on a stack array: `File::open`
    // allocates, which the child must not do.
    let mut fillers = [-1 as libc::c_int; 1024];
    let mut held = 0usize;
    let terminal = loop {
        let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            break child_errno();
        }
        if held == fillers.len() {
            child_fail(b"child: absurdly many fds\n");
        }
        fillers[held] = fd;
        held += 1;
    };
    if terminal != libc::EMFILE {
        child_fail(b"child: table did not fill with EMFILE\n");
    }

    // The failure this module replaces, pinned at the `accept` boundary:
    // with a connection pending and no fd to spend, `accept` fails EMFILE.
    // Smithay's source propagates that `Err` (`socket.accept()?`) out of
    // `process_events`, and calloop carries it out of `dispatch` and `run`
    // (both `?`), so `compositor::run` returns `Err` and the process exits
    // FAILURE -- the compositor-killing chain in `super`'s module doc. A
    // live pre-fix kill is also on record there (prlimit on a dev-VM
    // compositor); this pins the mechanism in the suite.
    match listener.accept() {
        Err(error) if error.raw_os_error() == Some(libc::EMFILE) => {}
        _ => child_fail(b"child: accept did not fail EMFILE while exhausted\n"),
    }
    // ...and the pending connection is still there: the next accept fails
    // the same way.
    match listener.accept() {
        Err(error) if error.raw_os_error() == Some(libc::EMFILE) => {}
        _ => child_fail(b"child: backlog not still pending with EMFILE\n"),
    }

    // The fixed loop: every turn either consumes a backlog entry or leaves,
    // so this returns -- it cannot spin and cannot propagate -- having shed
    // both pending clients. Nothing is served (serving means inserting the
    // client, which needs fds this table does not have); everything sheds.
    let mut served = 0;
    let end = drain(listener, spare, &mut |stream: UnixStream| {
        served += 1;
        drop(stream);
    });
    if !matches!(end, PostAction::Continue) {
        child_fail(b"child: drain left an exhausted listener deregistered\n");
    }
    if served != 0 {
        child_fail(b"child: served a connection with no free fd\n");
    }

    // Both shed clients see EOF with nothing before it -- there is no
    // protocol channel for a reason on a Wayland connection, the same wire
    // shape as the IPC shed. Read non-blocking: if a shed never happened
    // (a miscalibrated table -- see SERIAL), a blocking read would hang the
    // suite instead of failing the test.
    for fd in clients {
        let flags = unsafe { libc::fcntl(*fd, libc::F_GETFL) };
        if flags < 0 {
            child_fail(b"child: fcntl get failed\n");
        }
        if unsafe { libc::fcntl(*fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
            child_fail(b"child: fcntl set failed\n");
        }
    }
    let mut byte = [0u8; 1];
    for fd in clients {
        let mut total = 0;
        loop {
            let got = unsafe { libc::read(*fd, byte.as_mut_ptr().cast(), 1) };
            if got == 0 {
                break;
            }
            if got < 0 {
                child_fail(b"child: client not at EOF (never shed?)\n");
            }
            total += 1;
        }
        if total != 0 {
            child_fail(b"child: shed client got bytes before EOF\n");
        }
    }
    unsafe { libc::_exit(CHILD_PASS) };
}

#[test]
fn an_exhausted_listener_sheds_and_clears_the_backlog() {
    // Held from before the bind to after the final assertion: nothing else
    // in this module churns fds inside the count-to-fork window (see SERIAL).
    let _serial = serial();
    let (socket, path) = bound();
    let first = connect(&path);
    let second = connect(&path);
    // Before forking: the spare costs one fd, and the clients theirs.
    let spare = Spare::new();
    let usage = open_fd_count();
    let clients = [first.as_raw_fd(), second.as_raw_fd()];

    let child = unsafe { libc::fork() };
    assert!(child >= 0, "could not fork the exhaustion child");
    if child == 0 {
        exhausted_child(&socket, &spare, &clients, usage);
    }
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(child, &mut status, 0) },
        child,
        "could not wait for the exhaustion child"
    );
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == CHILD_PASS,
        "the exhausted child failed; its stderr above says where"
    );

    // The backlog the child shed is drained for this process too -- same
    // kernel backlog, shared across the fork. The next accept says
    // nothing-pending instead of failing, which is what clears a level
    // trigger.
    assert!(
        matches!(socket.accept(), Ok(None)),
        "the backlog is drained"
    );

    // Recovery is immediate once fds free up: no back-off, nothing disabled.
    // (The child lowered only its own copy of the limit; this process never
    // touched its own, so there is nothing to restore first.)
    let _fresh = connect(&path);
    let (end, served) = drain_all(&socket, &spare);
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(served, 1, "a post-recovery connection is served, not shed");
}

// --- pressure shed, through `admit` -----------------------------------------

/// A real compositor state to admit into (or shed before), with no backend:
/// nothing here renders, and nothing is dispatched either -- the peer
/// observation below needs no dispatch to tell the arms apart.
fn admit_state() -> (EventLoop<'static, State>, State) {
    let mut event_loop = EventLoop::try_new().expect("an event loop");
    let display = Display::new().expect("a wayland display");
    let state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        Keybindings::default(),
        Appearance::default(),
        1.0,
    )
    .expect("a compositor state");
    (event_loop, state)
}

/// Expects the peer of a shed stream: EOF with nothing before it. Retried
/// briefly rather than read once -- a local FIN lands immediately, but a
/// debug build on a loaded VM should not fail a correct shed on scheduling.
fn expect_eof(peer: UnixStream) {
    let mut peer = peer;
    peer.set_nonblocking(true).expect("a non-blocking peer");
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match peer.read(&mut byte) {
            Ok(0) => return,
            Ok(_) => panic!("a shed peer got bytes before EOF; no arm writes"),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "the peer is still open: the pressured newcomer was admitted, not shed"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("peer read failed: {error}"),
        }
    }
}

/// Expects the peer of an admitted stream: still open, with nothing to
/// read. A single read, not a wait: nothing is written before dispatch, so
/// an admitted peer has nothing to read by construction, and waiting would
/// only burn suite time proving a negative.
fn expect_held(peer: UnixStream) {
    let mut peer = peer;
    peer.set_nonblocking(true).expect("a non-blocking peer");
    let mut byte = [0u8; 1];
    match peer.read(&mut byte) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok(0) => panic!("the peer is at EOF: the calm newcomer was shed, not admitted"),
        Ok(_) => panic!("an admitted peer got bytes; no arm writes"),
        Err(error) => panic!("peer read failed: {error}"),
    }
}

#[test]
fn a_pressured_newcomer_gets_eof() {
    // A table with nothing free: the newcomer is shed before `insert_client`
    // ever sees it, and its peer reads EOF with no bytes first -- the same
    // wire shape as the `EMFILE` shed.
    let _serial = serial();
    let (_event_loop, mut state) = admit_state();
    let (server, peer) = UnixStream::pair().expect("a socket pair");
    admit(
        &mut state,
        server,
        Some(Table {
            used: 1024,
            soft: 1024,
        }),
    );
    expect_eof(peer);
}

#[test]
fn a_calm_table_admits() {
    // An observed table with headroom: the newcomer is inserted, and its
    // peer stays open.
    let _serial = serial();
    let (_event_loop, mut state) = admit_state();
    let (server, peer) = UnixStream::pair().expect("a socket pair");
    admit(
        &mut state,
        server,
        Some(Table {
            used: 14,
            soft: 1024,
        }),
    );
    expect_held(peer);
}

#[test]
fn an_unknown_table_admits() {
    // Fail open: a broken gauge must not deny innocents (the `EMFILE` shed
    // still catches real exhaustion underneath).
    let _serial = serial();
    let (_event_loop, mut state) = admit_state();
    let (server, peer) = UnixStream::pair().expect("a socket pair");
    admit(&mut state, server, None);
    expect_held(peer);
}
