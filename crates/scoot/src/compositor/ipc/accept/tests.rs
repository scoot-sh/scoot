//! Tests for the accept loop, against real listening sockets.
//!
//! The error-kind mapping ([`classify`](super::classify)) is pure and tested
//! with synthesized errors. Everything else goes through
//! [`drain`](super::drain) with a real bound listener, because what is under
//! test is the loop's effect on the backlog -- which only a real socket has.

use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use smithay::reexports::calloop::PostAction;

use super::super::listener;
use super::*;

/// A bound, non-blocking listener in a temp dir, with the dir kept alive.
fn bound() -> (tempfile::TempDir, UnixListener) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("scoot.sock");
    let socket = listener::bind(&path).expect("binds");
    (dir, socket)
}

fn connect(path: &Path) -> UnixStream {
    // A blocking `connect` on a unix socket returns once the connection is
    // queued in the listener's backlog, so by the time this returns the
    // server end can accept it -- no sleep, no retry.
    UnixStream::connect(path).expect("connects")
}

fn raw(errno: libc::c_int) -> io::Error {
    io::Error::from_raw_os_error(errno)
}

// --- error-kind coverage ---------------------------------------------------

#[test]
fn wouldblock_and_interrupted_mean_done_for_now() {
    for error in [
        io::Error::from(ErrorKind::WouldBlock),
        io::Error::from(ErrorKind::Interrupted),
        // And the errnos those kinds come from, since `accept` reports raw
        // OS errors rather than kinds.
        raw(libc::EAGAIN),
        raw(libc::EINTR),
    ] {
        assert!(
            matches!(classify(&error), Disposition::Done),
            "{error:?} must end the loop quietly"
        );
    }
}

#[test]
fn fd_exhaustion_means_shed() {
    for errno in [libc::EMFILE, libc::ENFILE] {
        let error = raw(errno);
        assert!(
            matches!(classify(&error), Disposition::Shed),
            "{error:?} must take the mitigation path"
        );
    }
}

#[test]
fn a_dead_listener_means_deregister() {
    for errno in [libc::EBADF, libc::EINVAL] {
        let error = raw(errno);
        assert!(
            matches!(classify(&error), Disposition::Dead),
            "{error:?} must deregister the source, not spin on it"
        );
    }
}

#[test]
fn anything_else_is_logged_and_left_registered() {
    // Neither done-for-now, nor exhaustion, nor a dead listener: loud, but
    // the control socket stays up. (A refused-over-cap connection never
    // reaches this function at all -- the cap applies to sockets `accept`
    // already returned, in `super::super::accept`.)
    for error in [
        raw(libc::EACCES),
        raw(libc::ENOMEM),
        io::Error::other("something unheard of"),
    ] {
        assert!(
            matches!(classify(&error), Disposition::Other),
            "{error:?} must not deregister the listener"
        );
    }
}

// --- the loop against a live listener --------------------------------------

#[test]
fn an_idle_listener_ends_cleanly_without_taking_anything() {
    let (_dir, socket) = bound();
    let spare = Spare::new();
    let mut taken = 0;
    let end = drain(&socket, &spare, &mut |stream| {
        taken += 1;
        drop(stream);
    });
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(taken, 0, "nothing was pending");
}

#[test]
fn pending_connections_are_served_then_the_loop_ends() {
    let (dir, socket) = bound();
    let path = dir.path().join("scoot.sock");
    let _first = connect(&path);
    let _second = connect(&path);
    let spare = Spare::new();
    let mut taken = 0;
    let end = drain(&socket, &spare, &mut |stream| {
        taken += 1;
        drop(stream);
    });
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(taken, 2, "every pending connection is taken");
    // The backlog is empty afterwards, which is what clears a level trigger:
    // the next accept says WouldBlock instead of reporting again.
    assert_eq!(
        socket.accept().expect_err("the backlog is drained").kind(),
        ErrorKind::WouldBlock
    );
}

#[test]
fn a_shed_that_finds_no_backlog_re_arms_the_spare() {
    // The disarm this pins (found by review): outer EMFILE spends the spare,
    // then the inner accept finds the backlog raced away -- WouldBlock on an
    // idle listener here -- and without a re-arm the mitigation stays
    // silently disarmed, so the *next* real exhaustion goes Stuck instead of
    // shedding. On the unfixed code the last assertion fails: the spare is
    // gone and nothing says so.
    let (_dir, socket) = bound(); // idle: nothing pending
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

#[test]
fn a_dead_listener_is_deregistered_not_spun_on() {
    // `EINVAL`: `accept` on a live fd that is not a listener -- one end of a
    // socket pair, which will never listen. `EBADF` (a closed listener fd)
    // reaches the same `Dead` arm by construction -- a single match arm over
    // both constants, covered per-errno by `a_dead_listener_means_deregister`
    // above -- but cannot be driven behaviorally: `from_raw_fd(-1)` is
    // rejected by std itself, and closing a live fd leaves a window where
    // another thread's new fd reuses the number mid-test.
    let (end_held, _peer) = UnixStream::pair().expect("a socket pair");
    let not_a_listener = unsafe { UnixListener::from_raw_fd(end_held.into_raw_fd()) };
    let spare = Spare::new();
    let mut taken = 0;
    let end = drain(&not_a_listener, &spare, &mut |stream| {
        taken += 1;
        drop(stream);
    });
    assert!(
        matches!(end, PostAction::Remove),
        "a non-listener must deregister rather than spin"
    );
    assert_eq!(taken, 0);
}

// --- fd exhaustion, in a forked child --------------------------------------
//
// `RLIMIT_NOFILE` is process-global: lowering it in-process fails unrelated
// tests running on other threads -- observed while writing this, as parallel
// closes freed slots and an accept "succeeded while exhausted". So the limit
// is lowered in a forked child, which inherits copies of every fd and its own
// copy of the limit: the parent's table is untouched, parallel tests cannot
// tell this ran at all, and there is nothing to restore afterwards.
//
// Discipline inside the child: after `fork` only the calling thread exists,
// and any lock another thread held stays held. The child therefore uses raw
// `libc` calls with stack buffers, plus the real `drain`/`shed_one`/
// `classify` path -- which is allocation- and lock-free by construction (see
// the module doc; `File::open` in particular must never appear there). No
// `assert!` either, since a panic would unwind through a half-frozen runtime:
// explicit checks that write to stderr and `_exit` with failure.

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
/// connected test clients. Never returns: pass exits 0, anything else writes
/// why to stderr and exits 1.
fn exhausted_child(
    listener: &UnixListener,
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

    // Fail-first, against the loop this replaces: `while let Ok(..)` leaves
    // on the first error having consumed nothing...
    let mut accepted_by_the_old_loop = 0;
    while let Ok((pending, _)) = listener.accept() {
        drop(pending);
        accepted_by_the_old_loop += 1;
    }
    if accepted_by_the_old_loop != 0 {
        child_fail(b"child: old loop accepted while exhausted\n");
    }
    // ...and the pending connection is still there: the next accept fails
    // the same way, which under `Mode::Level` is an immediate re-report and
    // a 100% CPU spin.
    match listener.accept() {
        Err(error) if error.raw_os_error() == Some(libc::EMFILE) => {}
        _ => child_fail(b"child: backlog not still pending with EMFILE\n"),
    }

    // The fixed loop: every turn either consumes a backlog entry or leaves,
    // so this returns -- it cannot spin -- having shed both pending clients.
    // Nothing is served (`take` would need an fd for the slot registration
    // the real callback does); everything here must shed.
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

    // Both shed clients see EOF with nothing before it -- unlike a
    // refused-over-cap connection, which gets its reason first (see
    // `the_connection_past_the_cap_is_refused_with_a_reason`: that path is
    // untouched, and stays distinguishable from this one on the wire as well
    // as in the logs).
    let mut byte = [0u8; 1];
    for fd in clients {
        let mut total = 0;
        loop {
            let got = unsafe { libc::read(*fd, byte.as_mut_ptr().cast(), 1) };
            if got == 0 {
                break;
            }
            if got < 0 {
                child_fail(b"child: client read failed\n");
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
    let (dir, socket) = bound();
    let path = dir.path().join("scoot.sock");
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
    // kernel backlog, shared across the fork. The next accept says WouldBlock
    // instead of failing, which is what clears a level trigger.
    assert_eq!(
        socket.accept().expect_err("the backlog is drained").kind(),
        ErrorKind::WouldBlock
    );

    // Recovery is immediate once fds free up: no back-off, nothing disabled.
    // (The child lowered only its own copy of the limit; this process never
    // touched its own, so there is nothing to restore first.)
    let _fresh = connect(&path);
    let mut served = 0;
    let end = drain(&socket, &spare, &mut |stream| {
        served += 1;
        drop(stream);
    });
    assert!(matches!(end, PostAction::Continue));
    assert_eq!(served, 1, "a post-recovery connection is served, not shed");
}
