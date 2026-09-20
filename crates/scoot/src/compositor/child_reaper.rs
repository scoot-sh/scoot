//! Reaping the children [`State::spawn`](super::State::spawn) starts.
//!
//! Every child scoot spawns -- the `--` startup command, every `spawn` bind,
//! every `scoot msg action spawn` -- used to stay in the process table as a
//! zombie from its exit until scoot itself exited: `spawn` dropped the
//! `std::process::Child` on the spot, and nothing installed a `SIGCHLD`
//! handler to reap it later. On a desktop that is one pid and one
//! `task_struct` per spawn; on the container this compositor names as a
//! deployment target, where cgroup `pids.max` counts zombies as live tasks,
//! a long agent-driven session ends at `fork: Resource temporarily
//! unavailable`. See `docs/backlog/resolved/spawned-children-never-reaped-done.md`
//! for the live `ps` evidence.
//!
//! The mechanism is a `sigaction` handler that writes to an eventfd the
//! event loop watches, and a drain that reaps exactly the pids `spawn`
//! tracked. Three designs were measured and rejected; this records why, so
//! nobody re-asks:
//!
//! - **`signal(SIGCHLD, SIG_IGN)` survives `execve`.** An ignored disposition
//!   is inherited across `exec`, so every child would start life unable to
//!   `wait()` for its own children (`ECHILD` instead of an exit status) --
//!   shells, `waybar` script modules, anything supervising a subprocess,
//!   broken untraceably. A *caught* handler, by contrast, is reset to
//!   `SIG_DFL` by `exec` for free, so it cannot leak into any child.
//! - **signalfd needs SIGCHLD blocked process-wide, and the mask also
//!   survives `exec`.** Measured on the dev VM: `std::process::Command`
//!   resets SIGPIPE only and inherits the mask untouched, and nothing clears
//!   it afterwards -- so every spawned child would start with SIGCHLD
//!   blocked and miss its own children's exits, unless each one reset the
//!   mask in a `pre_exec` closure. The handler here obliges no such thing:
//!   nothing is ever blocked.
//! - **A process-wide `waitpid(-1)` drain would reap children this process
//!   did not start.** The `scoot` unit-test binary's own fork/waitpid
//!   children (`ipc/accept/tests.rs`, `wayland_accept/tests.rs`) live in the
//!   same process an in-harness reaper test runs in; a `-1` drain would reap
//!   them first and fail their assertions -- green under `nextest` (one
//!   process per test), red under `cargo test` (shared). So the drain
//!   `waitpid`s each *tracked* pid instead, and only those.
//!
//! The wake is deliberately *not* calloop's own `ping`, though it is the same
//! shape (an eventfd drained by a level-triggered source, coalescing bursts
//! into one wakeup). `Ping`'s sender is opaque -- there is no raw fd for a
//! signal handler to write to -- and its `ping()` logs on error, which is
//! not something a handler may do. This owns the eventfd directly: [`install`]
//! keeps the write side as a process-lifetime fd named by an atomic, the
//! read side moves into a [`Generic`](smithay::reexports::calloop::generic::Generic)
//! source, and the handler does one 8-byte `write` and nothing else.
//!
//! This is reaping, not supervision: a dead child is collected and forgotten.
//! Restarting a bar that died or backing off a crash loop is a service
//! manager's job (see `docs/backlog/config/startup-programs-and-autostart.md`
//! for where that boundary is drawn).

#[cfg(test)]
mod tests;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use super::State;

/// The write side of the wake eventfd the `SIGCHLD` handler writes to, or
/// `-1` when no reaper is installed yet.
///
/// Always names an *open* fd while non-negative, which is what makes the
/// handler's blind `write` safe: [`install`] duplicates the eventfd it hands
/// to the loop and stores the duplicate here, closing the previous one only
/// after the swap, outside of any signal context. A signal landing between
/// the swap and that close writes to a still-open fd whose counter nobody
/// reads -- a lost wakeup for a loop that is going away, never a write into
/// a number the kernel has since handed to another file.
///
/// `Relaxed` ordering: the value is published once per install, long before
/// any child it could report on exists, and every later store only replaces
/// one valid fd with another.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Installs the `SIGCHLD` handler and wires its wakeup into `handle`'s loop,
/// then reaps anything that already exited so a signal that arrived mid-install
/// cannot leave a zombie behind with its wakeup already consumed.
///
/// Called once, from [`run`](super::run), before the first spawn: the startup
/// command, keybinding spawns and IPC spawns all flow through
/// [`State::spawn`](super::State::spawn), so one install covers every backend.
/// Repeating the call re-points the process-global handler at the newest loop
/// (last install wins); that is what the in-harness test relies on, and why
/// production must not do it more than once -- two live loops would split
/// wakeups between them.
///
/// Both eventfd ends are close-on-exec: a spawned child must not inherit the
/// wake pipe (pinned by
/// `activation/tests/spawn.rs::a_spawned_child_inherits_no_close_on_exec_fd`'s
/// shape, and by hygiene -- every child holding the write end open would keep
/// a stale counter reachable).
pub(super) fn install(
    handle: &LoopHandle<'static, State>,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: `eventfd` with these flags returns an owned fd on success, -1
    // with `errno` set on failure; nothing else in this process touches the
    // returned number before it is wrapped below.
    let read = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if read < 0 {
        return Err(Box::new(std::io::Error::last_os_error()));
    }
    // SAFETY: just returned, open, and owned -- the only copy in existence.
    let read = unsafe { OwnedFd::from_raw_fd(read) };

    // The duplicate the handler writes through (see `WAKE_FD` for why the
    // loop's own fd cannot serve: its number may be closed and reused while
    // the atomic still names it). `dup` clears close-on-exec, so the flag is
    // set back below before any spawn can observe it (nothing spawns between
    // here and the `fcntl`: this runs on the loop thread, before it starts
    // dispatching).
    //
    // SAFETY: `read` is open, so `dup` returns a fresh owned fd on success,
    // -1 with `errno` set on failure.
    let write = unsafe { libc::dup(read.as_raw_fd()) };
    if write < 0 {
        return Err(Box::new(std::io::Error::last_os_error()));
    }
    // SAFETY: `dup` just returned this fd; it is open and owned.
    let write = unsafe { OwnedFd::from_raw_fd(write) };
    let flags = unsafe { libc::fcntl(write.as_raw_fd(), libc::F_GETFD) };
    if flags >= 0 {
        unsafe {
            libc::fcntl(write.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }

    handle.insert_source(
        Generic::new(read, Interest::READ, Mode::Level),
        move |_, source, state: &mut State| {
            drain_wake(source.as_ref());
            state.reap_children();
            Ok(PostAction::Continue)
        },
    )?;

    // SAFETY: zeroed `sigaction`, an empty mask, and a function pointer are
    // always valid arguments; `sigaction` then only fails on a bad signal
    // number, and `SIGCHLD` is one. Still checked rather than assumed: a
    // reaper that silently never installed is the bug this module exists to
    // fix, returning as one.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_sigchld as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    action.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
    let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGCHLD, &action, &mut old) } != 0 {
        return Err(Box::new(std::io::Error::last_os_error()));
    }

    // Publish the write end only once the handler (above) and the drain
    // (the source) both exist, so a signal can never find one without the
    // other. The previous write end, if any, is closed *after* the swap:
    // a signal landing between the two writes to a still-open fd.
    let previous = WAKE_FD.swap(write.as_raw_fd(), Ordering::Relaxed);
    std::mem::forget(write);
    if previous >= 0 {
        unsafe {
            libc::close(previous);
        }
    }

    // Closes the install race the other way: a child that exited after its
    // signal was discarded (before the handler existed) or whose wakeup went
    // to a previous loop leaves no pending counter, so drain synchronously
    // once rather than trusting the first wakeup to arrive.
    state.reap_children();
    Ok(())
}

/// The `SIGCHLD` handler: one counter increment on the wake eventfd.
///
/// Async-signal-safe by construction -- an atomic load and a single `write`,
/// no heap, no locks, no logging. The return value is deliberately ignored:
/// `EAGAIN` means the counter is full, which itself means a wakeup is already
/// pending. `errno` is saved and restored so the interrupted code never sees
/// a spurious error from this write.
extern "C" fn on_sigchld(_signal: libc::c_int) {
    // SAFETY: `__errno_location` returns a valid pointer to this thread's
    // `errno` on Linux (glibc and musl alike); reading and writing through it
    // is async-signal-safe.
    let saved = unsafe { *libc::__errno_location() };
    let fd = WAKE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let increment: u64 = 1;
        unsafe {
            libc::write(
                fd,
                &increment as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }
    unsafe {
        *libc::__errno_location() = saved;
    }
}

/// Reads the wake eventfd back to zero after it fired.
///
/// One read always suffices: an eventfd read consumes the whole counter, so
/// there is no drain loop, and anything that exits between this read and the
/// drain below either leaves a zombie `reap_children` collects or a nonzero
/// counter the level trigger fires on again -- no lost wakeup either way.
/// Never fails the source: an unreadable wakeup retries on the next signal
/// rather than dropping the reaper.
fn drain_wake(source: &OwnedFd) {
    let mut counter: u64 = 0;
    loop {
        // SAFETY: `source` is the live read end of the wake eventfd, owned
        // by the loop source; the buffer is a valid 8-byte stack slot.
        let read = unsafe {
            libc::read(
                source.as_raw_fd(),
                &mut counter as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if read as usize == std::mem::size_of::<u64>() {
            return;
        }
        if read < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // Empty counter: a signal between the poll and this read is
            // already reflected above, so there is nothing to drain.
            if errno == libc::EAGAIN {
                return;
            }
            if errno == libc::EINTR {
                continue;
            }
            tracing::warn!(%errno, "child-reaper wakeup unreadable; retrying on the next SIGCHLD");
            return;
        }
        // A zero-length read on an eventfd cannot happen; a short one even
        // less so. Treat either as drained rather than looping forever.
        return;
    }
}

impl State {
    /// Reaps every tracked child that has exited, and forgets tracked pids
    /// that are gone without being reaped here.
    ///
    /// Runs on the loop thread, from the wake source after every `SIGCHLD`
    /// and once synchronously from [`install`]. Signals coalesce -- one
    /// `SIGCHLD` can stand for several exits -- so this always sweeps the
    /// whole tracked set rather than reaping once per wakeup.
    ///
    /// The empty-set fast path matters: every `SIGCHLD` in the process wakes
    /// the loop, including exits of children scoot never started (a test
    /// binary's own forks), and the common case is nothing tracked.
    pub(crate) fn reap_children(&mut self) {
        if self.spawned_children.is_empty() {
            return;
        }
        self.spawned_children.retain(|&pid| !reaped(pid));
    }
}

/// Whether `pid` -- a child [`State::spawn`](super::State::spawn) tracked --
/// is gone from the process table: reaped it just now, or already reaped
/// elsewhere (`ECHILD`, which drops the entry rather than leaking it).
///
/// `true` means "stop tracking". Only ever called with tracked pids, never
/// `-1`: reaping anything would steal the children the unit-test binary
/// forks for itself (see the module doc).
fn reaped(pid: u32) -> bool {
    let mut status = 0;
    loop {
        // SAFETY: `waitpid` with a positive pid and `WNOHANG` never blocks
        // and touches only that child; `status` is a valid out-pointer.
        let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if reaped > 0 {
            tracing::debug!(pid, status, "reaped a spawned child");
            return true;
        }
        if reaped == 0 {
            return false;
        }
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EINTR {
            continue;
        }
        if errno == libc::ECHILD {
            // Not ours to reap anymore (something else collected it, or it
            // was never our child): holding the pid would leak the entry,
            // and retrying would spin, so forget it.
            tracing::debug!(pid, "tracked child already reaped elsewhere; forgetting it");
            return true;
        }
        // Any other errno (`EINVAL` is impossible for a positive pid):
        // keep tracking and retry on the next wakeup rather than leak a
        // zombie by forgetting a child that is still ours.
        tracing::warn!(pid, %errno, "could not reap a spawned child; retrying on the next SIGCHLD");
        return false;
    }
}
