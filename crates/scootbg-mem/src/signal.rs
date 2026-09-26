//! Termination signals as a readable fd, for a `poll` loop.
//!
//! The daemon must remove its control socket on SIGTERM, SIGINT and SIGHUP
//! rather than die with it in place, and it waits in `poll`, not in a
//! signal handler. rustix has no `signalfd` and no safe `sigaction`, and a
//! handler written through its raw `kernel_sigaction` would need an
//! architecture-specific restorer. So this takes the thread route, which
//! needs no handler at all:
//!
//! 1. [`TerminationSignals::install`] blocks the three signals on the
//!    calling thread. Every thread spawned afterwards inherits that mask,
//!    so none of them can take the default action (death, socket left
//!    behind).
//! 2. It spawns one thread that waits for them with `rt_sigtimedwait`
//!    (`sigwait`), which dequeues a blocked signal synchronously.
//! 3. That thread writes the signal's number to a pipe; the read end is
//!    what the caller polls.
//!
//! **Call it first**, before any other thread exists: a thread spawned
//! earlier keeps the signals unblocked, and the kernel may deliver one to
//! it, killing the process the old way. The blocked mask is also inherited
//! across `execve`, so anything this process ever spawns must unblock them
//! first; the daemon spawns nothing today.
//!
//! The waiting thread costs no wakeups: it sleeps in the kernel until a
//! signal arrives.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::thread;

use rustix::io::Errno;
use rustix::pipe::{PipeFlags, pipe_with};
use rustix::runtime::{How, KernelSigSet, Signal, kernel_sigprocmask, kernel_sigwait};

#[cfg(test)]
mod tests;

/// The signals that ask the daemon to stop.
pub const TERMINATION: [Signal; 3] = [Signal::TERM, Signal::INT, Signal::HUP];

/// What the pipe carries when the waiting thread itself failed, so the
/// poll loop exits loudly instead of the daemon silently ignoring SIGTERM
/// for the rest of its life. `rt_sigtimedwait` without a timeout can only
/// fail with `EINTR` (retried), so this is not expected to happen.
pub const WAIT_FAILED: u8 = 0;

/// The read end of the signal pipe: readable once a termination signal has
/// arrived.
#[derive(Debug)]
pub struct TerminationSignals {
    read: OwnedFd,
}

fn termination_set() -> KernelSigSet {
    let mut set = KernelSigSet::empty();
    for signal in TERMINATION {
        set.insert(signal);
    }
    set
}

impl TerminationSignals {
    /// Blocks SIGTERM, SIGINT and SIGHUP on this thread and starts the
    /// thread that waits for them. See the module docs: call this before
    /// spawning any other thread.
    pub fn install() -> io::Result<Self> {
        let set = termination_set();
        // Both ends non-blocking: the reader drains without waiting, and
        // the writer must never block on a full pipe (a byte already in it
        // says the same thing).
        let (read, write) = pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK)?;

        // SAFETY: the set holds SIGTERM, SIGINT and SIGHUP only, none of
        // which glibc reserves for itself (it reserves 32 and 33 for its
        // thread machinery), so blocking them takes nothing from the C
        // runtime std links. Nothing in this process relies on their
        // default action or on a handler for them; blocking only defers
        // them to the `sigwait` below.
        unsafe { kernel_sigprocmask(How::BLOCK, Some(&set)) }?;

        let waited = set.clone();
        let spawned = thread::Builder::new()
            .name("scootbg-signals".into())
            // It only ever makes one syscall at a time.
            .stack_size(64 * 1024)
            .spawn(move || wait_forever(&waited, &write));
        if let Err(error) = spawned {
            // Unblock again so a failed install leaves the process as it
            // was: killable the default way.
            // SAFETY: as for the block above; unblocking restores the
            // default action for signals nothing else here depends on.
            let _ = unsafe { kernel_sigprocmask(How::UNBLOCK, Some(&set)) };
            return Err(error);
        }
        Ok(Self { read })
    }

    /// Drains the pipe and returns the last signal number read (or
    /// [`WAIT_FAILED`]), `None` if nothing was pending.
    pub fn take(&self) -> io::Result<Option<u8>> {
        let mut buf = [0u8; 16];
        let mut last = None;
        loop {
            match rustix::io::read(&self.read, &mut buf) {
                Ok(0) => return Ok(last),
                Ok(n) => last = buf.get(n - 1).copied(),
                Err(Errno::AGAIN) => return Ok(last),
                Err(Errno::INTR) => {}
                Err(errno) => return Err(errno.into()),
            }
        }
    }
}

impl AsFd for TerminationSignals {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }
}

/// The waiting thread: one byte into the pipe per signal, forever.
fn wait_forever(set: &KernelSigSet, write: &OwnedFd) {
    loop {
        // SAFETY: the same set as `install` blocked (no glibc-reserved
        // signal), and it is blocked on this thread, which inherited the
        // mask from the thread that blocked it before spawning this one:
        // `sigwait` requires exactly that.
        let byte = match unsafe { kernel_sigwait(set) } {
            // Signal numbers 1, 2 and 15: each fits a byte.
            Ok(signal) => signal.as_raw() as u8,
            Err(Errno::INTR) => continue,
            Err(_) => WAIT_FAILED,
        };
        // Non-blocking: a full pipe already carries a wake-up.
        let _ = rustix::io::write(write, &[byte]);
        if byte == WAIT_FAILED {
            return;
        }
    }
}
