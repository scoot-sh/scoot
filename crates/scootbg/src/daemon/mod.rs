//! `scootbg daemon`: the Wayland client and the control socket, in one
//! thread's `poll` loop.
//!
//! Start-up order matters:
//!
//! 1. The control socket is claimed (lock, stale check, bind), so a second
//!    daemon is refused before it touches the compositor, and clients that
//!    connect during start-up wait in the listen backlog.
//! 2. The Wayland connection is made and the globals bound.
//!
//! Then one thread, one `poll` over the Wayland fd, the listener and each
//! client, with no timeout: when nothing happens the daemon makes no system
//! call at all. No async runtime and no timers.
//!
//! `kill` (exit 0) and the compositor going away or a protocol error
//! (exit 1) remove the socket file on the way out. **Signals keep their
//! default action:** SIGTERM, SIGINT or SIGHUP kill the process on the spot
//! and leave the socket file behind. That is harmless by construction: the
//! kernel drops the daemon's `flock` with the process, clients get
//! `ECONNREFUSED` on the dead socket and report "not running", and the next
//! `scootbg daemon` sees the free lock and replaces the stale file. (Catching
//! signals would need `unsafe` rustix APIs that are hidden and unstable, as
//! rustix has no `signalfd`; see crate-and-daemon-done.md.)

mod crash;
mod respond;
mod wayland;

#[cfg(test)]
mod tests;

use std::fmt;
use std::io;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use wayland_client::backend::WaylandError as BackendError;

use crate::control::{Claim, ClaimError, Server};
use crate::output::warn;
use crate::paths::{self, PathError};
use respond::Responder;
use wayland::{Wayland, WaylandError};

/// Why the daemon stopped.
#[derive(Debug)]
pub enum Exit {
    /// `kill`: a clean, requested stop.
    Stopped,
    /// Anything else: printed, exit status 1.
    Failed(Error),
}

#[derive(Debug)]
pub enum Error {
    Paths(PathError),
    Claim(ClaimError),
    Wayland(WaylandError),
    /// The compositor closed the connection with nothing left to read.
    CompositorGone,
    /// The connection broke, or the compositor sent a last message (a
    /// protocol error) before closing it.
    Disconnected(BackendError),
    /// A protocol error, or a dispatch that failed.
    Dispatch(wayland_client::DispatchError),
    Poll(io::Error),
    /// The listener cannot accept any more (see `control`).
    Accept(io::Error),
    /// The spare fd could not be taken at start-up.
    Spare(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => write!(f, "{error}"),
            Self::Claim(error) => write!(f, "{error}"),
            Self::Wayland(error) => write!(f, "{error}"),
            Self::CompositorGone => {
                write!(
                    f,
                    "lost the connection to the compositor: it closed the connection"
                )
            }
            Self::Disconnected(error) => {
                write!(f, "lost the connection to the compositor: {error}")
            }
            Self::Dispatch(error) => write!(f, "Wayland error: {error}"),
            Self::Poll(error) => write!(f, "poll failed: {error}"),
            Self::Accept(error) => write!(f, "cannot accept clients any more: {error}"),
            Self::Spare(error) => write!(f, "cannot reserve a spare file descriptor: {error}"),
        }
    }
}

/// Runs the daemon until it is told to stop or cannot go on.
pub fn run() -> Exit {
    match serve() {
        Ok(()) => Exit::Stopped,
        Err(error) => Exit::Failed(error),
    }
}

fn serve() -> Result<(), Error> {
    let paths = paths::from_env().map_err(Error::Paths)?;
    let mut claim = Claim::acquire(&paths).map_err(Error::Claim)?;
    let armed = crash::install(paths.socket.clone());
    let server = Server::new(claim.listener()).map_err(Error::Spare)?;
    let (wayland, missing) = Wayland::connect().map_err(Error::Wayland)?;
    for interface in missing {
        warn(format_args!(
            "scootbg: note: the compositor has no {interface}; \
             scootbg will do without it"
        ));
    }
    let mut daemon = Daemon {
        wayland,
        server,
        responder: Responder::default(),
        wayland_wants_write: false,
        poll_fds: Vec::new(),
        revents: Vec::new(),
    };
    let result = daemon.run(&claim);
    // Socket and lock first, so a `kill` client that waits for its
    // connection to close finds the path free, and can start a new daemon,
    // once it does.
    // Disarm first: once `release` frees the lock another daemon may bind
    // this path, and a panic in what follows must not remove its socket.
    armed.disarm();
    claim.release();
    daemon.server.close_all();
    result
}

struct Daemon {
    wayland: Wayland,
    server: Server,
    responder: Responder,
    /// The last flush could not send everything: wait for POLLOUT.
    wayland_wants_write: bool,
    /// Reused across iterations so the loop allocates nothing once warm:
    /// emptied and re-typed each round (see `reuse`).
    poll_fds: Vec<PollFd<'static>>,
    revents: Vec<PollFlags>,
}

/// Slot order in the poll set.
const WAYLAND: usize = 0;
const LISTENER: usize = 1;

impl Daemon {
    fn run(&mut self, claim: &Claim) -> Result<(), Error> {
        loop {
            self.wayland.dispatch_pending().map_err(Error::Dispatch)?;
            self.flush_wayland()?;
            let Some(guard) = self.wayland.queue.prepare_read() else {
                // Events arrived for our queue meanwhile: dispatch them.
                continue;
            };

            let mut fds: Vec<PollFd<'_>> = reuse(std::mem::take(&mut self.poll_fds));
            let mut wayland_events = PollFlags::IN;
            if self.wayland_wants_write {
                wayland_events |= PollFlags::OUT;
            }
            let wayland_fd = guard.connection_fd();
            fds.push(PollFd::new(&wayland_fd, wayland_events));
            // Always: see `control`'s module docs for why the listener is
            // never dropped from the set.
            fds.push(PollFd::new(claim.listener(), PollFlags::IN));
            for conn in self.server.conns() {
                fds.push(PollFd::new(conn.stream(), conn.interest()));
            }

            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(Errno::INTR) => {
                    self.poll_fds = reuse(fds);
                    continue;
                }
                Err(errno) => return Err(Error::Poll(errno.into())),
            }
            self.revents.clear();
            self.revents.extend(fds.iter().map(PollFd::revents));
            self.poll_fds = reuse(fds);

            let wayland = self
                .revents
                .get(WAYLAND)
                .copied()
                .unwrap_or(PollFlags::empty());
            if wayland.intersects(PollFlags::OUT) {
                self.wayland_wants_write = false;
            }
            if wayland.intersects(PollFlags::HUP | PollFlags::ERR)
                && rustix::io::ioctl_fionread(guard.connection_fd()).unwrap_or(0) == 0
            {
                // The compositor is gone and sent nothing more. Stop here
                // rather than let the backend read the EOF: without its
                // `log` feature (which compiles C) it reports that error
                // with `eprintln!`, a stray line on every exit, and a panic
                // if stderr is a pipe whose reader has gone. Anything still
                // readable (a protocol error, say) is read below instead.
                return Err(Error::CompositorGone);
            }
            if wayland.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
                match guard.read() {
                    Ok(_) => {}
                    Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(Error::Disconnected(error)),
                }
            } else {
                drop(guard);
            }

            // Clients before accepting, so indices still match the poll set
            // (accepting may close the oldest client).
            let mut index = 0;
            for &revents in self.revents.get(LISTENER + 1..).unwrap_or_default() {
                // A closed client leaves the next one at the same index.
                if revents.is_empty() || self.server.service(index, revents, &mut self.responder) {
                    index += 1;
                }
            }
            if self.responder.stop {
                return Ok(());
            }
            if self
                .revents
                .get(LISTENER)
                .is_some_and(|r| r.intersects(PollFlags::IN))
            {
                self.server
                    .accept(claim.listener())
                    .map_err(Error::Accept)?;
            }
        }
    }

    fn flush_wayland(&mut self) -> Result<(), Error> {
        match self.wayland.conn.flush() {
            Ok(()) => Ok(()),
            Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                self.wayland_wants_write = true;
                Ok(())
            }
            Err(error) => Err(Error::Disconnected(error)),
        }
    }
}

/// Empties `fds` and hands its allocation back with a new lifetime.
///
/// A `PollFd` borrows the fd it watches, so a set built from this round's
/// borrows cannot be kept for the next. An empty `Vec`'s allocation can:
/// collecting an empty iterator in place reuses it (std's in-place
/// `collect` for `vec::IntoIter` through `filter_map`, the same size and
/// alignment on both sides), so the loop allocates nothing once warm.
// Not `filter`, which clippy suggests: that could not change the lifetime.
#[allow(clippy::unnecessary_filter_map)]
fn reuse<'a, 'b>(mut fds: Vec<PollFd<'a>>) -> Vec<PollFd<'b>> {
    fds.clear();
    fds.into_iter().filter_map(|_| None).collect()
}
