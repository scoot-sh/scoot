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
//! Then one thread, one `poll` over the Wayland fd, the listener, the image
//! worker's wake-up fd and each client, with no timeout: when nothing
//! happens the daemon makes no system call at all. No async runtime and no
//! timers; the one timeout there is exists only while the listener rests
//! after a failed accept (`listen`). Images are decoded and scaled on a
//! thread started per job (`worker`, `images`), never on this one.
//!
//! Zero outputs is a normal state (a headless session before its first
//! output, a laptop with the lid shut): nothing to draw, the same poll, no
//! wakeups, until an output's global arrives.
//!
//! A `set` or `clear` draws at once and is answered later, never by
//! blocking: each turn of the loop, after dispatching events, sends the
//! `wl_display.sync` for every change the outputs now show, and hands the
//! replies whose sync came back to their connections (`change`,
//! `crate::waiters`). A static wallpaper asks for no frame callbacks, so
//! once drawn it costs no wakeups.
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

mod canvas;
mod change;
mod config;
mod crash;
mod images;
mod listen;
mod respond;
mod restore;
mod surfaces;
mod wayland;
mod worker;

#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use wayland_client::backend::WaylandError as BackendError;

use crate::cli::DaemonOptions;
use crate::control::{Claim, ClaimError, Server};
use crate::paths::{self, PathError};
use crate::print::warn;
use change::Control;
use images::Images;
use listen::Listening;
use respond::{Responder, write_ready};
use wayland::{Wayland, WaylandError};

pub use config::Start;

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
    /// The spare fd could not be taken at start-up.
    Spare(io::Error),
    /// The image worker's wake-up fd could not be made.
    Worker(io::Error),
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
            Self::Spare(error) => write!(f, "cannot reserve a spare file descriptor: {error}"),
            Self::Worker(error) => write!(f, "cannot set up image decoding: {error}"),
        }
    }
}

/// How long the daemon waits on its way out for a state file write under
/// way (`crate::state::saver`). A write is a few hundred bytes; only a
/// stalled disk takes longer.
pub(super) const SAVE_GRACE: Duration = Duration::from_secs(2);

/// Runs the daemon until it is told to stop or cannot go on. `start` is
/// the section a daemon started by `apply-config` starts from
/// (`daemon::config`); without one it restores the profile's state (unless
/// `options.restore` is off).
pub fn run(options: DaemonOptions, start: Option<&Start>) -> Exit {
    match serve(options, start) {
        Ok(()) => Exit::Stopped,
        Err(error) => Exit::Failed(error),
    }
}

impl Error {
    /// Another daemon holds this display's lock: it won the race to start.
    pub fn lost_the_race(&self) -> bool {
        matches!(self, Self::Claim(ClaimError::AlreadyRunning { .. }))
    }
}

fn serve(options: DaemonOptions, start: Option<&Start>) -> Result<(), Error> {
    let paths = paths::from_env().map_err(Error::Paths)?;
    let mut claim = Claim::acquire(&paths).map_err(Error::Claim)?;
    let armed = crash::install(paths.socket.clone());
    let server = Server::new(claim.listener()).map_err(Error::Spare)?;
    let images = Images::new().map_err(Error::Worker)?;
    let (saved, record) = restore::load(options.profile);
    let (mut wayland, missing) = Wayland::connect(images, saved).map_err(Error::Wayland)?;
    match start {
        None => restore::apply(&mut wayland.state, record, options.restore),
        Some(start) => config::start(&mut wayland.state, record, start),
    }
    for interface in missing {
        warn(format_args!(
            "scootbg: note: the compositor has no {interface}; \
             scootbg will do without it"
        ));
    }
    let mut daemon = Daemon {
        wayland,
        server,
        listening: Listening::default(),
        wayland_wants_write: false,
        poll_fds: Vec::new(),
        revents: Vec::new(),
    };
    let result = daemon.run(&claim);
    // Before the socket goes: a `kill` client, answered once it does, then
    // finds the last change on disk.
    let state = &daemon.wayland.state;
    // Every writer, each given the grace (not `&&`, which would skip the
    // rest after one that ran out).
    let flushed = std::iter::once(&state.saved)
        .chain(&state.retired)
        .fold(true, |all, saved| saved.flush(SAVE_GRACE) & all);
    if !flushed {
        warn(format_args!(
            "scootbg: the state file was still being written after {} s; stopping \
             without waiting for it",
            SAVE_GRACE.as_secs()
        ));
    }
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
    listening: Listening,
    /// The last flush could not send everything: wait for POLLOUT.
    wayland_wants_write: bool,
    /// Reused across iterations so the loop allocates nothing once warm:
    /// emptied and re-typed each round (see `reuse`).
    poll_fds: Vec<PollFd<'static>>,
    revents: Vec<PollFlags>,
}

/// Slot order in the poll set; the clients follow.
const WAYLAND: usize = 0;
const LISTENER: usize = 1;
const WORKER: usize = 2;
const CLIENTS: usize = 3;

impl Daemon {
    fn run(&mut self, claim: &Claim) -> Result<(), Error> {
        loop {
            self.wayland.dispatch_pending().map_err(Error::Dispatch)?;
            // Replies nobody can receive any more are not waited for; this
            // is what bounds the waiting lists (`crate::waiters`).
            let server = &self.server;
            self.wayland
                .state
                .waiters
                .forget_gone(|conn| server.has(conn));
            // Replies first: a connection handed its reply goes on to the
            // requests queued behind it, and a `set` among them that
            // changes nothing sends the compositor nothing, so no event
            // would come back to wake the loop for it. Resolving after
            // delivering catches it in this same turn.
            if self.deliver_replies() {
                return Ok(());
            }
            change::progress(
                &mut self.wayland.state,
                &self.wayland.conn,
                &self.wayland.qh,
            );
            if images::pump(&mut self.wayland.state, &self.wayland.qh) {
                // A job failed to start and was landed: its reply goes out
                // before the loop sleeps.
                continue;
            }
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
            // Always, unless resting after a failed accept (see `listen`
            // and `control`'s module docs). Resting keeps its slot, with no
            // events asked for, so the indices below stay fixed.
            let (listen, timeout) = self.listening.poll_plan(Instant::now);
            let listener_events = if listen {
                PollFlags::IN
            } else {
                PollFlags::empty()
            };
            fds.push(PollFd::new(claim.listener(), listener_events));
            // Readable only once a job's result waits: no wakeups when idle.
            let worker_fd = self.wayland.state.images.worker.fd();
            fds.push(PollFd::new(&worker_fd, PollFlags::IN));
            for conn in self.server.conns() {
                fds.push(PollFd::new(conn.stream(), conn.interest()));
            }

            match poll(&mut fds, timeout.map(timespec).as_ref()) {
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

            if self
                .revents
                .get(WORKER)
                .is_some_and(|r| r.intersects(PollFlags::IN))
            {
                if let Some(done) = self.wayland.state.images.worker.take() {
                    images::land(&mut self.wayland.state, done, &self.wayland.qh);
                }
            }

            // Clients before accepting, so indices still match the poll set
            // (accepting may close the oldest client).
            let mut control = Control {
                state: &mut self.wayland.state,
                qh: &self.wayland.qh,
            };
            let mut responder = Responder::new(&mut control);
            let mut index = 0;
            for &revents in self.revents.get(CLIENTS..).unwrap_or_default() {
                // A closed client leaves the next one at the same index.
                if revents.is_empty() || self.server.service(index, revents, &mut responder) {
                    index += 1;
                }
            }
            if responder.stop {
                return Ok(());
            }
            if listen
                && self
                    .revents
                    .get(LISTENER)
                    .is_some_and(|r| r.intersects(PollFlags::IN))
            {
                self.accept(claim);
            }
        }
    }

    /// Hands every reply whose sync has come back to its connection, which
    /// then carries on with any request queued behind it. Returns whether
    /// one of those was a `kill`.
    fn deliver_replies(&mut self) -> bool {
        if self.wayland.state.ready.is_empty() {
            return false;
        }
        // Taken out so the handler can borrow the state; put back after,
        // emptied, so its allocation is kept.
        let mut ready = std::mem::take(&mut self.wayland.state.ready);
        let mut control = Control {
            state: &mut self.wayland.state,
            qh: &self.wayland.qh,
        };
        let mut responder = Responder::new(&mut control);
        for (conn, reply) in ready.drain(..) {
            self.server
                .complete(conn, |out| write_ready(out, &reply), &mut responder);
        }
        let stop = responder.stop;
        self.wayland.state.ready = ready;
        stop
    }

    /// Accepts waiting clients. A failure rests the listener rather than
    /// end the daemon, which would take the wallpaper with it (`listen`).
    fn accept(&mut self, claim: &Claim) {
        match self.server.accept(claim.listener()) {
            Ok(()) => {
                if self.listening.worked() {
                    warn(format_args!("scootbg: accepting clients again"));
                }
            }
            Err(error) => {
                if self.listening.failed(Instant::now()) {
                    warn(format_args!(
                        "scootbg: cannot accept clients: {error}; the wallpaper stays \
                         up, and accepting is retried every {} s",
                        listen::REST.as_secs()
                    ));
                }
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

/// A poll timeout. At most `listen::REST`, so it always fits; a second
/// is the fallback all the same, never a timeout of zero (a spin).
fn timespec(duration: Duration) -> rustix::event::Timespec {
    rustix::event::Timespec::try_from(duration).unwrap_or(rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    })
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
