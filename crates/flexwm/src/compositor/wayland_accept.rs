//! The Wayland accept loop: draining the listening socket without dying.
//!
//! This replaces Smithay's [`ListeningSocketSource`] with the same shape and
//! one load-bearing difference: what an `accept` error does. Smithay's source
//! drains with `while let Some(client) = socket.accept()?` (`socket.rs:114-121`
//! at the pinned rev), so any error but `WouldBlock` -- which
//! [`ListeningSocket::accept`] already folds to `Ok(None)` (`socket.rs:144-150`
//! in wayland-server 0.31.14) -- propagates as `Err` out of `process_events`,
//! through calloop's `dispatch_events` (`loop_logic.rs:527`, `?`), out of
//! `EventLoop::run` (`loop_logic.rs:657+`, `?`), out of `compositor::run`
//! (`mod.rs:193`, `?`), and `main` prints it and exits `FAILURE`. An `EMFILE`
//! on this socket does not take the socket down. It takes the whole
//! compositor down -- every client's unsaved state with it -- which is the
//! crash/hang severity class, and exactly what the per-connection fd bounds
//! make reachable: two connections at the 512-buffer cap hold ~2 x 513 fds
//! against a 1024-fd table (see `wl_buffers.rs`), each connection inside
//! every bound, nothing tripped, and the next backlog entry kills the
//! process. Found re-deriving the connection-cap ticket's arithmetic against
//! the current code; see `docs/backlog/resolved/wayland-connection-cap-done.md`.
//!
//! So this drains the way [`ipc::accept`] does: every error maps to a
//! [`PostAction`], and none ever propagates. `EMFILE`/`ENFILE` sheds one
//! pending connection per turn through the shared [`Spare`], a dead listener
//! deregisters, anything else logs loudly and leaves the listener registered.
//! What a shed client sees is an immediate EOF with no refusal line -- there
//! is no protocol channel for refusing a Wayland connection gracefully (the
//! ticket's own point), and no fd to serve even an out-of-band reason with,
//! the same wire shape as the IPC shed.
//!
//! Deliberately not a connection-count cap: any cap a real session fits
//! through (bar, panels, launcher, apps, dialogs -- dozens) still admits the
//! two greedy connections that fill the table, so a count would deny shells
//! on a miscount while stopping nothing. The record states the arithmetic.
//!
//! ## What may run in a forked child
//!
//! Same discipline as [`ipc::accept`]: the exhaustion test lowers
//! `RLIMIT_NOFILE` in a forked child, so [`drain`], [`shed_one`] and
//! everything they call must stay allocation- and lock-free. That is why the
//! shed-accepted socket *becomes* the new spare (via [`Spare::put`]) instead
//! of being dropped and reopened, and why the backlog-empty re-arm is a raw
//! `libc::open` on a static path, never `File::open`. Keep it that way.
//!
//! [`ListeningSocketSource`]: smithay::wayland::socket::ListeningSocketSource
//! [`ListeningSocket::accept`]: smithay::reexports::wayland_server::ListeningSocket::accept
//! [`ipc::accept`]: super::ipc::accept
//! [`Spare`]: super::ipc::accept::Spare
//! [`Spare::put`]: super::ipc::accept::Spare::put

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{
    EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory,
};
use smithay::reexports::wayland_server::{BindError, ListeningSocket};

use super::ipc::accept::{Disposition, ShedOutcome, Spare, classify};

#[cfg(test)]
mod tests;

/// A Wayland listening socket event source that survives `accept` errors.
///
/// [`Generic`] over the [`ListeningSocket`] gives registration for free; the
/// difference from Smithay's source is all in `process_events`, which drains
/// through [`drain`] below instead of propagating the first error. Owns its
/// [`Spare`]: one fd held for the session, the same cost as the IPC loop's.
/// (No `Debug`: [`Spare`] has none, and nothing requires it of a source.)
pub(super) struct WaylandListener {
    socket: Generic<ListeningSocket>,
    spare: Spare,
}

impl WaylandListener {
    /// Binds `wayland-1` through `wayland-32`, like Smithay's
    /// `ListeningSocketSource::new_auto` (which this replaces at the one call
    /// site): `wayland-0` is skipped because clients may connect to the wrong
    /// compositor, and everything downstream reads the name out of the
    /// environment this process sets after `State::new`.
    pub(super) fn bind_auto() -> Result<Self, BindError> {
        let socket = ListeningSocket::bind_auto("wayland", 1..33)?;
        Ok(Self {
            socket: Generic::new(socket, Interest::READ, Mode::Level),
            spare: Spare::new(),
        })
    }

    /// The name clients connect to, e.g. `wayland-1`.
    ///
    /// Always `Some`: [`ListeningSocket::bind_auto`] names what it binds.
    /// `expect` rather than a propagated error because a just-bound socket
    /// without a name is a backend bug, not an operator-facing failure.
    pub(super) fn socket_name(&self) -> OsString {
        self.socket
            .get_ref()
            .socket_name()
            .expect("a just-bound wayland socket has a name")
            .to_os_string()
    }
}

impl EventSource for WaylandListener {
    type Event = UnixStream;
    type Metadata = ();
    type Ret = ();
    type Error = io::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> io::Result<PostAction>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.socket.process_events(readiness, token, |_, socket| {
            Ok(drain(socket, &self.spare, &mut |stream| {
                callback(stream, &mut ())
            }))
        })
    }

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> smithay::reexports::calloop::Result<()> {
        self.socket.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> smithay::reexports::calloop::Result<()> {
        self.socket.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> smithay::reexports::calloop::Result<()> {
        self.socket.unregister(poll)
    }
}

/// Spends the spare to consume one backlog entry: close it, accept the pending
/// connection, and keep that connection's fd as the new spare.
///
/// Same shape as [`ipc::accept`]'s `shed_one`, over [`ListeningSocket`]
/// instead of `UnixListener` (whose `accept` yields an address this one does
/// not, and whose `WouldBlock` arrives as `Ok(None)` rather than `Err`).
/// Same fork-child discipline: no allocation, no locks -- the accepted socket
/// becomes the spare via [`Spare::put`], and only the backlog-empty path
/// reopens, as a raw `libc::open` on a static path.
fn shed_one(socket: &ListeningSocket, spare: &Spare) -> ShedOutcome {
    // The whole trick, as in `ipc::accept`: this close frees exactly one fd,
    // which is what the accept below spends. The table is process-global, so
    // another thread can still steal it in the window (since the screenshot
    // worker, anything doing `open`); narrow and self-resolving, but the shape
    // the `Stuck` corner has.
    drop(spare.take());
    match socket.accept() {
        Ok(Some(pending)) => {
            spare.put(File::from(OwnedFd::from(pending)));
            ShedOutcome::Consumed
        }
        // Nothing pending after all (or a signal got in first): the earlier
        // error raced a client going away. Re-arm the spent spare straight
        // away -- raw `open`, not `File::open`: see above -- and stay quiet.
        // (`ListeningSocket::accept` reports an empty backlog as `Ok(None)`;
        // `EINTR` arrives as `Err`, matched below.)
        Ok(None) => {
            re_arm(spare);
            ShedOutcome::BacklogEmpty
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            re_arm(spare);
            ShedOutcome::BacklogEmpty
        }
        Err(error) => ShedOutcome::Stuck(error),
    }
}

/// Re-arms the spare after a shed found no backlog, exactly like
/// [`ipc::accept`]'s: a raw `libc::open`, never `File::open`.
fn re_arm(spare: &Spare) {
    let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd >= 0 {
        // SAFETY: `open` just returned this fd; it is open and owned.
        spare.put(unsafe { File::from_raw_fd(fd) });
    } else {
        // The freed fd was stolen in the window (or `/dev/null` would not
        // open): disarmed, loudly rather than silently.
        tracing::warn!(
            "could not re-arm the wayland accept loop's spare fd; \
             further fd exhaustion will be logged but not shed"
        );
    }
}

/// Accepts until the backlog is empty, shedding through the spare on fd
/// exhaustion. Returns what the event-loop callback should return with:
///
/// - `Continue`: the backlog is drained (or was never the problem), and the
///   level trigger has nothing left to re-report -- the no-spin guarantee.
/// - `Remove`: the listener itself is dead; waking into the same failure
///   forever would be the spin this module exists to prevent.
///
/// Never `Err`: that propagation is the compositor-killing bug this module
/// replaces (see the module doc). The per-connection work -- inserting the
/// client, which can itself fail under id exhaustion -- stays the caller's:
/// `take` receives every accepted stream, and what it returns is not observed
/// here.
fn drain(socket: &ListeningSocket, spare: &Spare, take: &mut impl FnMut(UnixStream)) -> PostAction {
    loop {
        match socket.accept() {
            Ok(Some(stream)) => take(stream),
            Ok(None) => return PostAction::Continue,
            Err(error) => match classify(&error) {
                Disposition::Done => return PostAction::Continue,
                Disposition::Shed => match shed_one(socket, spare) {
                    ShedOutcome::Consumed => {
                        tracing::warn!(
                            "out of file descriptors; shed a pending wayland connection \
                             (exhaustion, not a cap: there is no protocol channel for \
                             refusing a wayland connection, so a shed one gets EOF)"
                        );
                    }
                    ShedOutcome::BacklogEmpty => return PostAction::Continue,
                    ShedOutcome::Stuck(error) => {
                        tracing::error!(
                            %error,
                            "wayland accept keeps failing with no fd to spend; \
                             leaving the backlog pending"
                        );
                        return PostAction::Continue;
                    }
                },
                Disposition::Dead => {
                    tracing::error!(%error, "wayland listener is dead; removing the accept source");
                    return PostAction::Remove;
                }
                Disposition::Other => {
                    tracing::error!(%error, "wayland accept failed; leaving the listener registered");
                    return PostAction::Continue;
                }
            },
        }
    }
}
