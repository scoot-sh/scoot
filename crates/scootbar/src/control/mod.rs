//! The control socket's server half: accepting, bounding and servicing
//! connections, all non-blocking, from the daemon's single `poll` loop.
//!
//! At most [`MAX_CONNECTIONS`] clients at once. When one more arrives, the
//! oldest is closed to admit it, so clients that connect and never send
//! (or never read) cannot lock out a `scootbar msg kill`, and the daemon needs
//! no timers to find them.
//!
//! **The listener is always polled.** It is level-triggered, so a
//! connection that cannot be accepted keeps it readable, and the loop must
//! either accept it or stop. Out of file descriptors (`EMFILE`/`ENFILE`),
//! the server frees one it owns and accepts the waiting client, so `kill`
//! still gets through:
//!
//! 1. close the oldest client, the same policy as above; else
//! 2. close the spare, a `dup` of the listener taken at start-up (a dup,
//!    not a path such as `/dev/null`, so it cannot be missing), and
//!    retake it once a client closes;
//! 3. with neither, the process owns nothing it can free, and retrying at
//!    once would spin: [`Server::accept`] returns an error.
//!
//! Any other `accept` error that is not about one connection (`ENOMEM`,
//! `ENOBUFS`, ...) is returned the same way, for the same reason. The
//! daemon does not exit on it, since that would take the bar with
//! it: the listener *rests* for a second, polled for no events, and is then
//! tried again (`daemon::listen`), so the loop neither spins nor goes deaf
//! for good.
//!
//! (As scootbg's `control/mod.rs`, minus its deferred replies: every
//! request here is answered at once, so there is no `complete`.)

mod claim;
mod conn;
pub mod framing;
pub mod paths;
pub mod protocol;

pub mod client;

#[cfg(test)]
mod subscribe_tests;
#[cfg(test)]
mod tests;

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixListener;

use rustix::event::PollFlags;
use rustix::io::Errno;

pub use claim::{Claim, ClaimError};
pub use conn::{Conn, Handler, Kinds, Status};

/// Subscribers at once, at most: a subscriber costs nothing while nothing
/// happens but is one write per event while something does, and an agent
/// needs one or two. One more is refused with an error that says so.
pub const MAX_SUBSCRIBERS: usize = 4;

/// Clients served at once. `scootbar msg` commands are one request each,
/// so more than a handful at a time means something is stuck or hostile.
pub const MAX_CONNECTIONS: usize = 16;

/// Size of the read scratch buffer shared by every connection.
const SCRATCH: usize = 4096;

pub struct Server {
    conns: Vec<Conn>,
    /// How many of `conns` are subscribed: the loop asks every turn, so it
    /// is kept, not counted.
    subscribers: usize,
    scratch: Box<[u8; SCRATCH]>,
    /// One fd held back for when the process is out of them (see the
    /// module docs). `None` only while it has been spent and not yet
    /// retaken.
    spare: Option<OwnedFd>,
}

impl Server {
    /// A server for `listener`, holding a dup of it as the spare fd.
    pub fn new(listener: &UnixListener) -> io::Result<Self> {
        Ok(Self::with_spare(Some(spare_for(listener)?)))
    }

    /// A server with the given spare (or none, as after it was spent).
    pub fn with_spare(spare: Option<OwnedFd>) -> Self {
        Self {
            conns: Vec::with_capacity(MAX_CONNECTIONS),
            subscribers: 0,
            scratch: Box::new([0; SCRATCH]),
            spare,
        }
    }

    pub fn conns(&self) -> &[Conn] {
        &self.conns
    }

    /// How many connections are subscribed.
    pub fn subscribers(&self) -> usize {
        self.subscribers
    }

    fn recount(&mut self) {
        self.subscribers = self
            .conns
            .iter()
            .filter(|conn| conn.subscription().is_some())
            .count();
    }

    /// Sends `bytes` (whole event lines) to every subscriber of `kind`, in
    /// one write each; one that cannot take it all is closed. Nothing is
    /// buffered.
    pub fn broadcast(&mut self, kind: protocol::EventKind, bytes: &[u8]) {
        if self.subscribers == 0 || bytes.is_empty() {
            return;
        }
        self.conns.retain_mut(|conn| match conn.subscription() {
            Some(kinds) if kinds.wants(kind) => conn.send_event(bytes) == Status::Keep,
            _ => true,
        });
        self.recount();
    }

    #[cfg(test)]
    pub fn has_spare(&self) -> bool {
        self.spare.is_some()
    }

    /// Accepts waiting clients: at most [`MAX_CONNECTIONS`] per call, so a
    /// flood cannot hold the loop. An error means accepting again at once
    /// would only repeat it (see the module docs): the daemon rests the
    /// listener rather than exit (`daemon::listen`).
    pub fn accept(&mut self, listener: &UnixListener) -> io::Result<()> {
        if self.spare.is_none() {
            // Spent earlier; retake it now if an fd has come free. Only
            // tried while degraded, so it costs nothing normally.
            self.spare = spare_for(listener).ok();
        }
        for _ in 0..MAX_CONNECTIONS {
            match listener.accept() {
                Ok((stream, _)) => self.admit(stream),
                Err(e) => match classify(&e) {
                    AcceptError::Drained => return Ok(()),
                    AcceptError::Retry => {}
                    AcceptError::OutOfFds => {
                        // Linux reserves the new fd before dequeuing, so
                        // EMFILE comes back even with nobody waiting:
                        // free one only for a client that is there.
                        if !pending(listener) {
                            return Ok(());
                        }
                        if !self.free_an_fd() {
                            return Err(io::Error::new(
                                e.kind(),
                                format!(
                                    "out of file descriptors with none to free, \
                                     so no client can be accepted: {e}"
                                ),
                            ));
                        }
                    }
                    AcceptError::Fatal => return Err(e),
                },
            }
        }
        Ok(())
    }

    fn admit(&mut self, stream: std::os::unix::net::UnixStream) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        if self.conns.len() >= MAX_CONNECTIONS {
            // The oldest goes; `Vec` order is accept order. (A subscriber
            // is as old as it is: a flood of connects ages it out, as it
            // does any idle client.)
            self.conns.remove(0);
        }
        self.conns.push(Conn::new(stream));
        self.recount();
    }

    /// Closes the oldest client, else the spare. `false` when there is
    /// nothing left to close.
    fn free_an_fd(&mut self) -> bool {
        if !self.conns.is_empty() {
            self.conns.remove(0);
            self.recount();
            true
        } else {
            self.spare.take().is_some()
        }
    }

    /// Services connection `index` for `revents`, dropping it if done.
    /// Returns whether it was kept (so the caller's index stays valid).
    pub fn service<H: Handler>(
        &mut self,
        index: usize,
        revents: PollFlags,
        handler: &mut H,
    ) -> bool {
        let Some(conn) = self.conns.get_mut(index) else {
            return false;
        };
        let kept = match conn.service(revents, &mut self.scratch[..], handler) {
            Status::Keep => true,
            Status::Close => {
                self.conns.remove(index);
                false
            }
        };
        self.recount();
        kept
    }

    /// Shutdown: one non-blocking attempt to send what each client is
    /// owed (a `kill`'s reply), then close them all.
    pub fn close_all(&mut self) {
        for conn in &mut self.conns {
            conn.flush_once();
        }
        self.conns.clear();
    }
}

/// What an `accept` error means for the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcceptError {
    /// Nothing more is waiting.
    Drained,
    /// That one connection failed; it is gone from the queue, try the next.
    Retry,
    /// `EMFILE`/`ENFILE`: free an fd and try again.
    OutOfFds,
    /// Not about one connection; retrying would spin.
    Fatal,
}

fn classify(error: &io::Error) -> AcceptError {
    let Some(errno) = error.raw_os_error().map(Errno::from_raw_os_error) else {
        return AcceptError::Fatal;
    };
    match errno {
        Errno::AGAIN => AcceptError::Drained,
        // accept(2): pending network errors on the new socket, reported
        // by accept itself, "should be treated like EAGAIN by retrying";
        // the failed connection is dequeued either way.
        Errno::INTR
        | Errno::CONNABORTED
        | Errno::PROTO
        | Errno::PERM
        | Errno::NETDOWN
        | Errno::NOPROTOOPT
        | Errno::HOSTDOWN
        | Errno::NONET
        | Errno::HOSTUNREACH
        | Errno::OPNOTSUPP
        | Errno::NETUNREACH => AcceptError::Retry,
        Errno::MFILE | Errno::NFILE => AcceptError::OutOfFds,
        _ => AcceptError::Fatal,
    }
}

/// Whether a connection is waiting on `listener` (a zero-timeout poll).
/// An error counts as waiting, so the caller frees an fd rather than
/// return with a connection possibly stuck.
fn pending(listener: &UnixListener) -> bool {
    let mut fds = [rustix::event::PollFd::new(listener, PollFlags::IN)];
    let zero = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    match rustix::event::poll(&mut fds, Some(&zero)) {
        Ok(n) => n > 0,
        Err(_) => true,
    }
}

/// The spare fd: a dup of the listener, which needs no path to exist.
fn spare_for(listener: &UnixListener) -> io::Result<OwnedFd> {
    listener.try_clone().map(OwnedFd::from)
}
