//! A session held across the bus coming and going: the lifecycle the
//! tray and the media module share (one copy: the tray's older one was
//! moved onto this).
//!
//! Two states. **Waiting** owns an inotify fd on the bus socket's
//! directory (and, only after the bus kept dropping the bar, a one-shot
//! retry timer) and holds nothing else: a machine with no session bus
//! costs one descriptor and no wakeups. **Live** owns the connection, set
//! up by the consumer's `start` into its own state `S`; a dead connection
//! drops `S` whole (a bus that went away takes every pending call and
//! every name the consumer tracked with it) and dials once more at once,
//! so a transient death heals in the same turn. A bus that takes the bar
//! in and drops it at once, [`MAX_QUICK_DEATHS`] times in a row, is left
//! alone for [`RETRY_AFTER_QUICK_DEATHS`] (a one-shot timer, not a poll),
//! so a bus that dislikes something the consumer sends cannot spin the
//! bar.
//!
//! The consumer works events through a closure, so this knows nothing of
//! what is spoken. Work per wake is bounded ([`MAX_PUMPS_PER_WAKE`] pumps
//! of [`conn::MAX_EVENTS_PER_TURN`] events): a flood backs up in the
//! socket and is worked a wake at a time, the bar's other sources between.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::PollFlags;
use rustix::fs::inotify::{CreateFlags, WatchFlags};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use super::conn::{self, Conn, Event};

#[cfg(test)]
mod tests;

/// How many [`Conn::pump`]s one wake of the bus socket makes (64 events
/// each).
const MAX_PUMPS_PER_WAKE: usize = 4;
/// A connection that dies sooner than this after it was made is a quick
/// death; [`MAX_QUICK_DEATHS`] in a row and the bar stops redialling.
const QUICK_DEATH: Duration = Duration::from_secs(5);
const MAX_QUICK_DEATHS: u8 = 3;
/// After [`MAX_QUICK_DEATHS`] the bar tries once more this long on (and
/// again, each time one try dies quick): short in tests.
#[cfg(not(test))]
const RETRY_AFTER_QUICK_DEATHS: Duration = Duration::from_secs(30);
#[cfg(test)]
const RETRY_AFTER_QUICK_DEATHS: Duration = Duration::from_millis(300);

/// What a live session is to the link: the connection it owns.
pub trait Session {
    fn conn(&self) -> &Conn;
    fn conn_mut(&mut self) -> &mut Conn;
}

/// How to reach the bus.
#[derive(Debug)]
pub enum Addr {
    /// The socket's filesystem path.
    Path(PathBuf),
    /// The bus's address names nothing this client dials: never
    /// connects, shows nothing, holds no descriptor.
    Unusable,
    /// An already-open stream (the tests' socketpair end).
    #[cfg(test)]
    Stream(UnixStream),
}

enum Bus<S> {
    Waiting {
        /// The directory watch, when it could be armed.
        notify: Option<OwnedFd>,
        /// Armed only after the bus kept dropping us.
        retry: Option<OwnedFd>,
    },
    Live(Box<S>),
}

/// The link: where the bus is, and the wait or the live session.
pub struct Link<S: Session> {
    /// For the lines said on stderr: `media`.
    who: &'static str,
    /// For the same lines: `session bus`, or `system bus` for BlueZ.
    bus_name: &'static str,
    path: PathBuf,
    bus: Bus<S>,
    /// Builds the session on a fresh connection.
    start: fn(Conn) -> S,
    /// When the live connection was made: a bus that kills it at once is
    /// told from one that went away.
    since: Option<Instant>,
    /// Connections in a row that died within [`QUICK_DEATH`].
    quick_deaths: u8,
}

impl<S: Session> Link<S> {
    /// Starts the link: dials now when the bus is there (a local socket:
    /// the blocking `Hello` is two round trips), or waits on it when it is
    /// not.
    pub fn start(who: &'static str, addr: Addr, start: fn(Conn) -> S) -> Self {
        Self::start_on(who, "session bus", addr, start)
    }

    /// As [`Link::start`], on the named bus (`system bus` for BlueZ: only
    /// the lines said on stderr differ).
    pub fn start_on(
        who: &'static str,
        bus_name: &'static str,
        addr: Addr,
        start: fn(Conn) -> S,
    ) -> Self {
        let (path, stream, dial): (_, Option<UnixStream>, _) = match addr {
            Addr::Path(path) => (path, None, true),
            Addr::Unusable => (PathBuf::new(), None, false),
            #[cfg(test)]
            // No path to redial: a stream the tests hand over is the only
            // bus there is, and a real session bus on the machine running
            // them must not be mistaken for it.
            Addr::Stream(stream) => (
                PathBuf::from("/nonexistent/scootbar-test-bus"),
                Some(stream),
                true,
            ),
        };
        let mut link = Self {
            who,
            bus_name,
            path,
            bus: Bus::Waiting {
                notify: None,
                retry: None,
            },
            start,
            since: None,
            quick_deaths: 0,
        };
        match stream {
            Some(stream) => link.connected(stream),
            None if dial => link.connect(),
            None => {}
        }
        link
    }

    /// The live session, if there is one.
    pub fn live(&self) -> Option<&S> {
        match &self.bus {
            Bus::Live(live) => Some(live),
            Bus::Waiting { .. } => None,
        }
    }

    pub fn live_mut(&mut self) -> Option<&mut S> {
        match &mut self.bus {
            Bus::Live(live) => Some(live),
            Bus::Waiting { .. } => None,
        }
    }

    /// How many sources [`Link::watch`] adds now: the consumer's own
    /// numbers start after them.
    pub fn source_count(&self) -> usize {
        match &self.bus {
            Bus::Live(_) => 1,
            Bus::Waiting { notify, retry } => {
                usize::from(notify.is_some()) + usize::from(retry.is_some())
            }
        }
    }

    /// Hands `add` the link's fds, in source order: the bus socket while
    /// live (with `OUT` only while a write waits or staged messages wait
    /// their turn, so the poll returns at once and the work goes on a wake
    /// at a time), the directory watch and the retry timer while waiting.
    /// No other timer, ever: an idle link wakes nothing.
    pub fn watch<'fd>(&'fd self, add: &mut dyn FnMut(BorrowedFd<'fd>, PollFlags)) {
        match &self.bus {
            Bus::Live(live) => {
                let conn = live.conn();
                let mut flags = PollFlags::IN;
                if conn.want_write() || conn.has_staged_work() {
                    flags |= PollFlags::OUT;
                }
                add(conn.as_fd(), flags);
            }
            Bus::Waiting { notify, retry } => {
                if let Some(notify) = notify {
                    add(notify.as_fd(), PollFlags::IN);
                }
                if let Some(retry) = retry {
                    add(retry.as_fd(), PollFlags::IN);
                }
            }
        }
    }

    /// One of the link's sources is ready (`source` is below
    /// [`Link::source_count`]): works the bus's events through `work`,
    /// which says whether it changed what the consumer shows, or handles
    /// the wait. Says whether anything shown changed (a dropped session
    /// always did: what it held is gone).
    pub fn on_ready(
        &mut self,
        source: usize,
        events: PollFlags,
        work: &mut dyn FnMut(&mut S, Event) -> bool,
    ) -> bool {
        match &self.bus {
            Bus::Live(_) => self.on_bus(events, work),
            Bus::Waiting { notify, retry } => {
                let retry_source = usize::from(notify.is_some());
                if retry.is_some() && source == retry_source {
                    self.on_retry()
                } else {
                    self.on_notify(events)
                }
            }
        }
    }

    /// One turn on the bus: pumps the connection and works each event,
    /// re-pumping while capped, up to [`MAX_PUMPS_PER_WAKE`].
    fn on_bus(&mut self, events: PollFlags, work: &mut dyn FnMut(&mut S, Event) -> bool) -> bool {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return self.drop_live();
        }
        let Bus::Live(live) = &mut self.bus else {
            return false;
        };
        let mut changed = false;
        let mut dead = live.conn().dead();
        'pumps: for _ in 0..MAX_PUMPS_PER_WAKE {
            if dead {
                break;
            }
            let (batch, capped) = live.conn_mut().pump();
            for event in batch {
                changed |= work(live, event);
                if live.conn().dead() {
                    dead = true;
                    break 'pumps;
                }
            }
            if live.conn().dead() {
                dead = true;
                break;
            }
            if !capped {
                break;
            }
        }
        if dead {
            return self.drop_live() || changed;
        }
        changed
    }

    /// Waits on the bus socket's directory: the socket's own directory
    /// when there is one, else the runtime directory. A watch that cannot
    /// be armed is nothing polled: a reload starts over (the volume
    /// module's rule).
    fn wait(&mut self) {
        let dir = self
            .path
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(crate::control::paths::runtime_dir);
        let notify = rustix::fs::inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .ok()
            .and_then(|fd| {
                rustix::fs::inotify::add_watch(
                    &fd,
                    &dir,
                    WatchFlags::CREATE
                        | WatchFlags::MOVED_TO
                        | WatchFlags::DELETE_SELF
                        | WatchFlags::MOVE_SELF,
                )
                .ok()
                .map(|_| fd)
            });
        self.bus = Bus::Waiting {
            notify,
            retry: None,
        };
    }

    /// Drops the session (what it held is gone with the bus), then dials
    /// once more at once: a transient death with a live bus heals in the
    /// same turn, and a bus truly gone falls back to waiting. One attempt
    /// only, so a refusing bus cannot hot-loop the bar. Always `true`: the
    /// consumer's view changed (it held something, or the death costs one
    /// redraw).
    fn drop_live(&mut self) -> bool {
        let quick = self
            .since
            .take()
            .is_some_and(|at| at.elapsed() < QUICK_DEATH);
        self.quick_deaths = if quick {
            self.quick_deaths.saturating_add(1)
        } else {
            0
        };
        self.wait();
        if self.quick_deaths >= MAX_QUICK_DEATHS {
            // A bus that takes us in and drops us at once, again and again:
            // dialling forever would spin the bar. Try once more later.
            if self.quick_deaths == MAX_QUICK_DEATHS {
                self.quick_deaths = self.quick_deaths.saturating_add(1);
                crate::print::warn(format_args!(
                    "scootbar: {}: the bus keeps dropping the connection; \
                     trying again in {} s, or when it is restarted",
                    self.who,
                    RETRY_AFTER_QUICK_DEATHS.as_secs().max(1)
                ));
            }
            self.arm_retry();
        } else {
            self.connect();
        }
        true
    }

    /// Connects a stream that is already open (a dialled bus, or the
    /// tests' socketpair end).
    #[cfg_attr(not(test), allow(dead_code))]
    fn connected(&mut self, stream: UnixStream) {
        match conn::setup(stream) {
            Ok(conn) => self.adopt(conn),
            Err(_) => self.wait(),
        }
    }

    /// Dials the bus; a failure waits, said on stderr so an empty module
    /// names its reason.
    fn connect(&mut self) {
        match conn::connect(&self.path) {
            Ok(conn) => self.adopt(conn),
            Err(error) => {
                crate::print::warn(format_args!(
                    "scootbar: {}: no {} ({error}); waiting for one",
                    self.who, self.bus_name
                ));
                self.wait();
            }
        }
    }

    fn adopt(&mut self, conn: Conn) {
        self.since = Some(Instant::now());
        self.bus = Bus::Live(Box::new((self.start)(conn)));
    }

    /// Arms the one-shot retry for a bus that kept dropping us.
    fn arm_retry(&mut self) {
        let Bus::Waiting { retry, .. } = &mut self.bus else {
            return;
        };
        let Ok(fd) = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        ) else {
            return;
        };
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: RETRY_AFTER_QUICK_DEATHS.as_secs() as i64,
                tv_nsec: i64::from(RETRY_AFTER_QUICK_DEATHS.subsec_nanos()),
            },
        };
        if timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).is_ok() {
            *retry = Some(fd);
        }
    }

    /// The retry fired: one more dial, and one more quick death puts the
    /// link back to waiting (the count is left one short of the limit).
    fn on_retry(&mut self) -> bool {
        if let Bus::Waiting { retry, .. } = &mut self.bus {
            if let Some(timer) = retry.take() {
                let mut expirations = [0u8; 8];
                let _ = rustix::io::read(&timer, &mut expirations);
            }
        }
        self.quick_deaths = MAX_QUICK_DEATHS - 1;
        self.connect();
        false
    }

    /// Handles the directory watch: drains it (else it stays ready) and
    /// dials when the socket's name arrived. Events for other names cost
    /// one scan of the bytes read, never a dial.
    fn on_notify(&mut self, events: PollFlags) -> bool {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            self.wait();
            return false;
        }
        let Bus::Waiting { notify, .. } = &self.bus else {
            return false;
        };
        let mut arrived = false;
        let mut failed = false;
        if let Some(fd) = notify {
            let mut buf = [0u8; 4096];
            loop {
                match rustix::io::read(fd, &mut buf) {
                    Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
                    Ok(n) => arrived |= scan_names(&buf[..n], bus_name(&self.path)),
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
        }
        if failed {
            self.wait();
            return false;
        }
        if arrived {
            // The socket was made anew: whatever dropped us before is
            // over, so the count starts again.
            self.quick_deaths = 0;
            self.connect();
        }
        false
    }
}

/// The bus socket's file name, for the watch scan.
fn bus_name(path: &Path) -> &[u8] {
    path.file_name()
        .map(|name| name.as_encoded_bytes())
        .unwrap_or(b"bus")
}

/// Whether an inotify buffer holds an event for `want`: whole-field
/// matching on the file name, so a longer name cannot false-positive. A
/// buffer that does not frame (a short header or name) answers `true`:
/// probing the socket once too often costs a dial, missing it costs the
/// bus.
fn scan_names(buf: &[u8], want: &[u8]) -> bool {
    let mut rest = buf;
    if rest.is_empty() {
        return false;
    }
    loop {
        if rest.len() < 16 {
            return true;
        }
        let len = u32::from_ne_bytes([rest[12], rest[13], rest[14], rest[15]]) as usize;
        rest = &rest[16..];
        if rest.len() < len {
            return true;
        }
        let (name, tail) = rest.split_at(len);
        // The name is NUL-padded in the event: compare without the padding.
        let name = name.split(|byte| *byte == 0).next().unwrap_or(&[]);
        if name == want {
            return true;
        }
        rest = tail;
        if rest.is_empty() {
            return false;
        }
    }
}
