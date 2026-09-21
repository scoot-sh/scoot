//! Reloading the config on `SIGHUP`, the second trigger alongside IPC.
//!
//! The IPC `reload` request is the first trigger (see `reload.rs`); this is
//! the Unix-standard explicit counterpart: `kill -HUP <pid>` and
//! `systemctl reload`-shaped tooling work without a socket client. inotify
//! stays out deliberately -- file-watching a live-edited config fires
//! mid-keystroke, while a HUP says exactly when.
//!
//! The mechanism mirrors [`super::child_reaper`]: a `sigaction` handler that
//! writes one counter increment to an eventfd the loop watches, and a drain
//! that does the real work on the loop thread. The handler never parses TOML
//! (or anything else): it wakes the loop, and the loop drives the same
//! shared path the IPC request drives -- same validate-before-apply, same
//! applied/refused semantics, same TTY VT-bind guard, same applies-under-lock
//! decision. They ride along for free because they live in that path, which
//! is the point of sharing it rather than forking it.
//!
//! Composing with the reaper, by construction: each owns its own signal (this
//! one `SIGHUP`, the reaper `SIGCHLD`), its own `sigaction`, its own eventfd
//! and its own calloop source. Neither names anything the other touches, so
//! a HUP and a CHLD in either order wake their own source and run their own
//! drain -- no interference is representable, not merely unobserved (pinned
//! by `tests::sighup_and_sigchld_do_not_interfere`).
//!
//! Reentrancy, for the same reason: everything runs on the loop's one thread.
//! A HUP that lands mid-reload leaves a nonzero eventfd counter, and the
//! level-triggered source fires again once the callback returns -- so rapid
//! HUP+HUP is either coalesced into one reload (both signals before the
//! drain) or sequential (two dispatches), and both are safe: `reload` diffs
//! against live state, so the second of two sees the first's results and
//! reports two empty lists. A HUP racing an IPC `reload` funnels into the
//! same `State::reload` on the same thread for the same reason -- the two
//! can interleave only at dispatch boundaries, never inside each other.
//!
//! Children keep default HUP semantics for free: a *caught* handler is reset
//! to `SIG_DFL` by `exec`, so nothing leaks into a spawned child -- unlike
//! `SIG_IGN`, which survives `exec` and would break children's own HUP
//! handling (shells in terminals rely on default HUP-to-SIGHUP delivery, and
//! `nohup` relies on observing it). Nothing is ever blocked either (no
//! signalfd mask to inherit). Pinned by
//! `tests::a_spawned_child_sees_default_sighup_and_an_empty_mask`.
//!
//! Only the compositor installs this handler. `scootctl` gets none: a HUP to
//! the client keeps its default meaning (terminate), which is the only sane
//! one for a short-lived process with no config to reload.
//!
//! This is a trigger, not supervision: a failed reload (unreadable,
//! malformed, unknown field) keeps the running config and logs loudly, never
//! exits -- the shared path's failure semantics, unchanged.

#[cfg(test)]
mod tests;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use super::State;

/// The write side of the wake eventfd the `SIGHUP` handler writes to, or
/// `-1` when no handler is installed yet.
///
/// The same shape as the reaper's `WAKE_FD`, for the same reason: the loop's
/// own fd cannot serve, because its number may be closed and reused while the
/// atomic still names it. [`install`] duplicates the eventfd it hands to the
/// loop and stores the duplicate here, closing the previous one only after
/// the swap, outside of any signal context.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Installs the `SIGHUP` handler and wires its wakeup into `handle`'s loop.
///
/// Called once, from [`run`](super::run), next to the child reaper's own
/// install: the default disposition (terminate) must be fully replaced before
/// the session serves anything, because a HUP killing the compositor would be
/// a session-loss bug -- the inverse of this feature. Repeating the call
/// re-points the process-global handler at the newest loop (last install
/// wins); that is what the in-harness tests rely on, and why production must
/// not do it more than once.
///
/// Both eventfd ends are close-on-exec: a spawned child must not inherit the
/// wake pipe.
pub(super) fn install(
    handle: &LoopHandle<'static, State>,
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

    // The duplicate the handler writes through (see `WAKE_FD`). `dup` clears
    // close-on-exec, so the flag is set back below before any spawn can
    // observe it (nothing spawns between here and the `fcntl`: this runs on
    // the loop thread, before it starts dispatching).
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
            state.reload_from_sighup();
            Ok(PostAction::Continue)
        },
    )?;

    // SAFETY: zeroed `sigaction`, an empty mask, and a function pointer are
    // always valid arguments; `sigaction` then only fails on a bad signal
    // number, and `SIGHUP` is one. Still checked rather than assumed: a
    // reload trigger that silently never installed is a HUP away from killing
    // the session, returning as one.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_sighup as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    action.sa_flags = libc::SA_RESTART;
    let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGHUP, &action, &mut old) } != 0 {
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

    // No synchronous reload here, unlike the reaper's install-time drain:
    // there is no install race to close (a HUP that arrived before the
    // handler existed kept its default disposition and killed the process --
    // loudly, not silently -- so there is no swallowed wakeup to recover),
    // and a reload with no prior HUP would just re-read an unchanged file.
    Ok(())
}

/// The `SIGHUP` handler: one counter increment on the wake eventfd.
///
/// Async-signal-safe by construction -- an atomic load and a single `write`,
/// no heap, no locks, no logging. The return value is deliberately ignored:
/// `EAGAIN` means the counter is full, which itself means a wakeup is already
/// pending. `errno` is saved and restored so the interrupted code never sees
/// a spurious error from this write.
extern "C" fn on_sighup(_signal: libc::c_int) {
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
/// there is no drain loop, and a HUP between this read and the reload below
/// either leaves a nonzero counter the level trigger fires on again (a second
/// sequential reload) or none at all (coalesced) -- no lost wakeup either
/// way. Never fails the source: an unreadable wakeup retries on the next HUP
/// rather than dropping the trigger.
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
            tracing::warn!(%errno, "sighup wakeup unreadable; retrying on the next SIGHUP");
            return;
        }
        // A zero-length read on an eventfd cannot happen; a short one even
        // less so. Treat either as drained rather than looping forever.
        return;
    }
}

impl State {
    /// Serves a `SIGHUP`: the same shared path as `Request::Reload`, minus
    /// the reply, which has nowhere to go.
    ///
    /// Runs on the loop thread, from the wake source after every `SIGHUP`.
    /// [`State::reload`] already logs the
    /// applied/refused summary (`info!("config reloaded")`) and any failure
    /// (`error!("config reload failed")`), so dropping the `Response` here
    /// loses nothing an operator tailing the log would see -- and every
    /// semantic the shared path owns (validate-before-apply, the TTY VT-bind
    /// guard, applying under lock) rides along untouched.
    fn reload_from_sighup(&mut self) {
        let _ = self.reload();
    }
}
