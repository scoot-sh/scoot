//! The poll loop's Wayland primitives: what every shell app's daemon loop
//! runs around its own sources.
//!
//! One `poll` over the Wayland fd and whatever else the daemon watches,
//! with no timeout while idle. Three pieces of that are the same for every
//! app, so they live here; the poll set itself (modules, control clients,
//! launched commands) is each daemon's own:
//!
//! - [`flush`]: send what the loop queued, remembering a full socket.
//! - [`connection_gone`]: the hang-up with nothing left to read, which
//!   ends the daemon.
//! - [`read_after_poll`]: take what the poll reported on the Wayland fd.
//!
//! Moved from `scootbar`'s `daemon` loop (`scootui-extract-wayland`) with
//! nothing else changed: the errors map to each daemon's own at its call
//! site.

use std::io;

use rustix::event::PollFlags;
use wayland_client::Connection;
use wayland_client::backend::{ReadEventsGuard, WaylandError as BackendError};

/// Send what the loop queued. A full socket (`WouldBlock`) is not an
/// error: `wants_write` is set, and the next turn polls for `OUT`.
pub fn flush(conn: &Connection, wants_write: &mut bool) -> Result<(), BackendError> {
    match conn.flush() {
        Ok(()) => Ok(()),
        Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
            *wants_write = true;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Whether the compositor is gone: a hang-up or error on the Wayland fd
/// with nothing left to read. Stop here rather than let the backend read
/// the EOF, which without its `log` feature it reports with `eprintln!`.
/// Anything still readable (a protocol error) is read by
/// [`read_after_poll`] instead.
pub fn connection_gone(guard: &ReadEventsGuard, revents: PollFlags) -> bool {
    revents.intersects(PollFlags::HUP | PollFlags::ERR)
        && rustix::io::ioctl_fionread(guard.connection_fd()).unwrap_or(0) == 0
}

/// Take what the poll reported on the Wayland fd: read when it says
/// readable, hung up or errored, else drop the guard unread. A read that
/// finds nothing after all (`WouldBlock`) is not an error.
pub fn read_after_poll(guard: ReadEventsGuard, revents: PollFlags) -> Result<(), BackendError> {
    if revents.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
        match guard.read() {
            Ok(_) => Ok(()),
            Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    }
}
