//! The control socket's server half: accepting, bounding and servicing
//! connections, all non-blocking, from the daemon's single `poll` loop.
//!
//! At most [`MAX_CONNECTIONS`] clients at once. When one more arrives, the
//! oldest is closed to admit it, so clients that connect and never send
//! (or never read) cannot lock out a `scootbg kill`, and the daemon needs
//! no timers to find them.

mod claim;
mod conn;

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io;
use std::os::unix::net::UnixListener;

use rustix::event::PollFlags;

pub use claim::{Claim, ClaimError};
pub use conn::{Conn, Handler, Status};

/// Clients served at once. `scootbg` commands are one request each, so
/// more than a handful at a time means something is stuck or hostile.
pub const MAX_CONNECTIONS: usize = 16;

/// Size of the read scratch buffer shared by every connection.
const SCRATCH: usize = 4096;

pub struct Server {
    conns: Vec<Conn>,
    scratch: Box<[u8; SCRATCH]>,
    /// One fd kept open for the case where the process is out of them: it
    /// is closed to accept (and immediately drop) a waiting client, so the
    /// listener does not stay readable and spin the loop. Reopened after.
    spare: Option<File>,
}

impl Server {
    pub fn new() -> Self {
        Self {
            conns: Vec::with_capacity(MAX_CONNECTIONS),
            scratch: Box::new([0; SCRATCH]),
            spare: open_spare(),
        }
    }

    pub fn conns(&self) -> &[Conn] {
        &self.conns
    }

    /// Whether the listener should be polled. Without a spare fd, a
    /// connection we cannot accept would keep it readable forever; it is
    /// left unpolled until a spare can be reopened.
    pub fn listening(&mut self) -> bool {
        if self.spare.is_none() {
            self.spare = open_spare();
        }
        self.spare.is_some()
    }

    /// Accepts waiting clients: at most [`MAX_CONNECTIONS`] per call, so a
    /// flood cannot hold the loop.
    pub fn accept(&mut self, listener: &UnixListener) {
        for _ in 0..MAX_CONNECTIONS {
            match listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(true).is_err() {
                        continue;
                    }
                    if self.conns.len() >= MAX_CONNECTIONS {
                        // The oldest goes; `Vec` order is accept order.
                        self.conns.remove(0);
                    }
                    self.conns.push(Conn::new(stream));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                    ) => {}
                Err(e) if is_fd_exhaustion(&e) => {
                    // Out of fds: spend the spare on the waiting client and
                    // turn it away, rather than leave it queued.
                    self.spare = None;
                    drop(listener.accept());
                    self.spare = open_spare();
                    return;
                }
                Err(_) => return,
            }
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
        match conn.service(revents, &mut self.scratch[..], handler) {
            Status::Keep => true,
            Status::Close => {
                self.conns.remove(index);
                false
            }
        }
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

fn is_fd_exhaustion(error: &io::Error) -> bool {
    let errno = error
        .raw_os_error()
        .map(rustix::io::Errno::from_raw_os_error);
    matches!(
        errno,
        Some(rustix::io::Errno::MFILE | rustix::io::Errno::NFILE)
    )
}

fn open_spare() -> Option<File> {
    File::open("/dev/null").ok()
}
