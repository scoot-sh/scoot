//! The bar feed: native push to the bar's `notifications` push module.
//!
//! On every change of DND or of the unread count, and only then, the
//! daemon connects to the bar's control socket
//! (`$XDG_RUNTIME_DIR/scootbar-<display>.sock`) and sends
//! `set notifications <payload>`. The payload is the shape
//! `scoot-notify-sync` sends (`nix/modules/notifications-home.nix`):
//!
//! - DND: `{text: "DND n" | "DND", class: "muted", tooltip, icon?}`;
//! - unread: `{text: "n", class: "urgent" | "normal", tooltip, icon?}`,
//!   where `urgent` means at least one counted critical;
//! - idle: `{text: ""}`.
//!
//! The counted `n` is live cards plus unseen history: a card on screen
//! counts live; one held by DND or a rule counts unseen. Icons ride
//! per update from `bar_icon_unread` / `bar_icon_dnd` (empty sends
//! none, leaving the key out, exactly as the script does).
//!
//! A bar that is not running fails the send, and the owed send is
//! retried: by the socket watch when the bar is absent (nothing polls),
//! and by a bounded, capped re-dial while the bar stays reachable but
//! unanswering. The push itself never blocks the loop: `sync` opens the
//! socket non-blocking and queues the line, the daemon's poll set drives
//! the in-flight push (connect, write, reply) beside the bus under a
//! deadline that clears loaded turn periods, so a stuck bar costs one fd and one wakeup per
//! change, never a stall of the bus service. A module that is not
//! placed answers an error, and the daemon logs the guidance once per
//! run. No `unwrap`, `expect` or panic on any path: every refusal
//! degrades to a line and an owed send.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::PollFlags;

use crate::model::dnd::Dnd;
use crate::model::store::Store;

/// The bar module the feed writes: the `push` module id the Nix side
/// places (`nix/modules/scootbar.nix`), unchanged by this ticket.
pub const MODULE_ID: &str = "notifications";

/// How long one in-flight push waits on the bar (connect, write and
/// reply) before the loop reaps it owed: long enough that a
/// slow-but-healthy bar converges on the first attempt (a 3 s-delayed
/// reply still lands), short enough that a stuck bar holds one fd for
/// a bounded spell. It never stalls the loop: the push rides the
/// existing poll set beside the bus, so expiries, control and the bus
/// are served every turn while it waits.
///
/// Thirty seconds, not five: the deadline must clear the loop's worst
/// turn period under load, not its median. CI showed control-plane
/// round trips past 5 s on loaded runners (the bus `until` allows 10 s
/// for the same reason); a shorter deadline reaps pushes whose reply
/// is already on its way, and no retry schedule converges while every
/// attempt outlives the same bound. Async makes the length free: a
/// stuck bar holds one fd and costs one wakeup, never a stall. The
/// scale mirrors the control server's `STALL_DEADLINE`.
const PUSH_DEADLINE: Duration = Duration::from_secs(30);

/// How soon an owed-but-reachable send re-dials: a transient push
/// failure (a past-deadline reply under load) converges on its own,
/// without waiting for the next change. Bounded by `RETRY_CAP`, so a
/// silent bar costs about one wakeup a second for a bounded spell,
/// then parks on the watch until it rebinds or the next change.
const RETRY_AFTER: Duration = Duration::from_secs(1);

/// Timer re-dials per owed spell past the first attempt: a silent bar
/// is retried this often, then left to the watch and the next change.
/// Every re-dial is silent and at least `RETRY_AFTER` apart: no log,
/// no spin. Four, not more: with a 30 s deadline each cycle is half a
/// minute, so four already covers a two-minute wedge; transients need
/// one or two.
const RETRY_CAP: u32 = 4;

/// Grace past a refused dial before the race re-dial: binds create
/// the socket file before they listen, so the first dial can lose
/// that race. One silent re-dial closes it; a second refusal is a
/// stale socket, owed on the watch. A poll timeout, never a sleep:
/// the loop serves everything else meanwhile.
const RACE_AFTER: Duration = Duration::from_millis(25);

/// Compressed timings for the unit-test pins below: the same machine
/// with a fast deadline and retry, so the slow/silent-bar pins run in
/// about a second rather than a minute.
#[cfg(test)]
const TEST_DEADLINE: Duration = Duration::from_millis(500);
#[cfg(test)]
const TEST_RETRY_AFTER: Duration = Duration::from_millis(50);
#[cfg(test)]
const TEST_RETRY_CAP: u32 = 2;

/// The longest control reply read. Replies are one line (`ok` or an
/// error); this only bounds a daemon gone wrong.
const MAX_REPLY: u64 = 64 * 1024;

/// What the feed counts: the mode plus the bar-visible numbers. The
/// daemon pushes on a change of this, and only then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// Do-not-disturb: changes the payload's shape, not just its count.
    pub dnd: Dnd,
    /// Live cards plus unseen history.
    pub unread: usize,
    /// Whether any counted card is critical.
    pub critical: bool,
}

impl State {
    /// Reads the bar-visible numbers from the store. No allocation:
    /// the daemon calls this every turn.
    pub fn of(store: &Store) -> Self {
        Self {
            dnd: store.dnd(),
            unread: store.feed_count(),
            critical: store.feed_has_critical(),
        }
    }
}

/// Builds the exact `scoot-notify-sync` payload for `state`: the
/// script's `jq` shape, key for key. Empty icons leave the key out, as
/// the script's `with_icon` does. No I/O; allocates only the returned
/// value (called on change, never per frame).
pub fn payload(state: State, icon_unread: &str, icon_dnd: &str) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut object = Map::new();
    if state.dnd.holds() {
        object.insert(
            "text".to_owned(),
            Value::String(if state.unread > 0 {
                format!("DND {}", state.unread)
            } else {
                String::from("DND")
            }),
        );
        object.insert("class".to_owned(), Value::String(String::from("muted")));
        object.insert(
            "tooltip".to_owned(),
            Value::String(if state.unread > 0 {
                format!(
                    "{} notifications held by do-not-disturb -- click to let them through",
                    state.unread
                )
            } else {
                String::from("do-not-disturb is on -- click to let notifications through")
            }),
        );
        if !icon_dnd.is_empty() {
            object.insert("icon".to_owned(), Value::String(icon_dnd.to_owned()));
        }
    } else if state.unread > 0 {
        object.insert(
            "text".to_owned(),
            Value::String(format!("{}", state.unread)),
        );
        object.insert(
            "class".to_owned(),
            Value::String(String::from(if state.critical {
                "urgent"
            } else {
                "normal"
            })),
        );
        object.insert(
            "tooltip".to_owned(),
            Value::String(format!(
                "{} notifications -- click to hold them with do-not-disturb",
                state.unread
            )),
        );
        if !icon_unread.is_empty() {
            object.insert("icon".to_owned(), Value::String(icon_unread.to_owned()));
        }
    } else {
        object.insert("text".to_owned(), Value::String(String::new()));
    }
    Value::Object(object)
}

/// Where the bar's socket lives: the environment's, or a fixed path
/// the tests hand over (a scratch socket, never the process
/// environment).
#[derive(Debug)]
enum Target {
    Env,
    /// The unit tests hand over a scratch socket, never the process
    /// environment.
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(PathBuf),
}

/// The feed: the last state placed on the bar, whether a send is
/// owed, the in-flight non-blocking push, and the one-shot watch for
/// the bar's socket appearing.
pub struct Feed {
    target: Target,
    icon_unread: String,
    icon_dnd: String,
    /// The last state placed (or refused as not-placed): `None` until
    /// the startup baseline, which counts as placed without sending.
    last: Option<State>,
    /// A send is owed: the last attempt found no bar, or the bar went
    /// quiet past the connect. Retried on the next change, when the
    /// watch fires, or on the bounded timer while reachable.
    owed: bool,
    /// The not-placed guidance already logged this run.
    placed_logged: bool,
    /// The inotify watch on the runtime dir, armed only while owed,
    /// with the socket file name it scans for.
    watch: Option<Watch>,
    /// The in-flight push, opened without blocking and driven by
    /// `service` on later turns. At most one: a newer change
    /// supersedes it (last-wins; the bar converges on the latest).
    pending: Option<Pending>,
    /// A scheduled re-dial while owed: the race grace or the bounded
    /// retry. Sets the poll timeout; `None` while nothing is owed and
    /// while owed-with-no-bar (the watch alone: zero wakeups).
    redial: Option<Redial>,
    /// Timer re-dials used in the current owed spell (the race grace
    /// and event-driven starts never consume it).
    retries: u32,
    /// The last failure saw a socket (past the connect, a write, or
    /// an unanswering bar) rather than an absence: only then does a
    /// timer re-dial make sense (an absent bar can only reappear,
    /// which the watch reports).
    reachable: bool,
    /// The push deadline, retry interval and cap: the production
    /// constants above, compressed under test so the pins run fast.
    push_deadline: Duration,
    retry_after: Duration,
    retry_cap: u32,
}

/// One armed watch: the inotify fd plus the socket file name whose
/// appearance retires it.
struct Watch {
    fd: OwnedFd,
    /// The socket's final component, as bytes.
    name: Vec<u8>,
}

/// An in-flight push: opened without blocking, driven by `drive` on
/// later turns, reaped past `deadline`. Never blocks the loop: the
/// bus is served every turn while this waits.
struct Pending {
    stream: UnixStream,
    /// The request bytes and how many went out.
    line: Vec<u8>,
    written: usize,
    phase: Phase,
    /// The reply bytes so far (up to the first newline).
    read: Vec<u8>,
    /// Give up at: the bar stayed reachable but unanswering.
    deadline: Instant,
    /// This attempt speaks for its change: its failure logs. Only the
    /// spell's first attempt (and the race grace it spawns) is loud;
    /// every scheduled re-dial is silent.
    loud: bool,
    /// This attempt is the race grace: a refusal parks rather than
    /// scheduling another.
    race: bool,
}

/// Where one push waits: the daemon's poll interest follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Connect,
    Write,
    Read,
}

/// A scheduled re-dial while owed.
enum Redial {
    /// The bind-listen race grace: one silent re-dial past a refused
    /// dial, at `RACE_AFTER`. A second refusal is a stale socket.
    Race(Instant),
    /// The bounded retry while the bar stays reachable but
    /// unanswering, at `retry_after`. Capped by `retry_cap`, then the
    /// spell parks on the watch.
    Retry(Instant),
}

/// One non-blocking step of the in-flight push.
enum Step {
    /// It would block: sleep until the poll (or the deadline) says more.
    Wait,
    /// Forward progress (a phase advanced, bytes moved): keep driving.
    More,
    /// The attempt finished.
    Done(Outcome),
}

impl Feed {
    /// A feed pushing to the environment's bar socket, with `config`'s
    /// icons.
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            target: Target::Env,
            icon_unread: config.bar_icon_unread.clone(),
            icon_dnd: config.bar_icon_dnd.clone(),
            last: None,
            owed: false,
            placed_logged: false,
            watch: None,
            pending: None,
            redial: None,
            retries: 0,
            reachable: false,
            push_deadline: PUSH_DEADLINE,
            retry_after: RETRY_AFTER,
            retry_cap: RETRY_CAP,
        }
    }

    /// A feed pushing to `socket` (the unit tests' fake bar), with the
    /// given icons.
    #[cfg(test)]
    pub fn for_test(socket: PathBuf, icon_unread: &str, icon_dnd: &str) -> Self {
        Self {
            target: Target::Fixed(socket),
            icon_unread: icon_unread.to_owned(),
            icon_dnd: icon_dnd.to_owned(),
            last: None,
            owed: false,
            placed_logged: false,
            watch: None,
            pending: None,
            redial: None,
            retries: 0,
            reachable: false,
            push_deadline: TEST_DEADLINE,
            retry_after: TEST_RETRY_AFTER,
            retry_cap: TEST_RETRY_CAP,
        }
    }

    /// Takes `config`'s icons (`msg reload`): the next change pushes
    /// with them.
    pub fn set_icons(&mut self, config: &crate::config::Config) {
        self.icon_unread.clone_from(&config.bar_icon_unread);
        self.icon_dnd.clone_from(&config.bar_icon_dnd);
    }

    /// Baselines without sending: the startup state counts as placed,
    /// so the first push waits for the first change (a bar started
    /// later shows its placeholder until then).
    pub fn baseline(&mut self, store: &Store) {
        self.last = Some(State::of(store));
    }

    /// Whether a push is owed: the last attempt found no bar.
    pub fn owed(&self) -> bool {
        self.owed
    }

    /// The watch fd for the daemon's poll set: `None` unless a send is
    /// owed.
    pub fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        self.watch.as_ref().map(|watch| watch.fd.as_fd())
    }

    /// The in-flight push's fd and what it waits for, for the daemon's
    /// poll set: `OUT` while connecting or writing, `IN` while reading
    /// the reply (plus hangup and error either way). `None` while no
    /// push is in flight.
    pub fn pending_poll(&self) -> Option<(BorrowedFd<'_>, PollFlags)> {
        let pending = self.pending.as_ref()?;
        let mut flags = match pending.phase {
            Phase::Connect | Phase::Write => PollFlags::OUT,
            Phase::Read => PollFlags::IN,
        };
        flags |= PollFlags::HUP | PollFlags::ERR;
        Some((pending.stream.as_fd(), flags))
    }

    /// How long the daemon's poll may sleep before the feed needs a
    /// turn: the in-flight push's deadline, else the scheduled
    /// re-dial. `None` at idle and while owed-with-no-bar, so neither
    /// costs a wakeup.
    pub fn poll_timeout(&self) -> Option<Duration> {
        if let Some(pending) = self.pending.as_ref() {
            return Some(pending.deadline.saturating_duration_since(Instant::now()));
        }
        match self.redial.as_ref() {
            Some(Redial::Race(at)) | Some(Redial::Retry(at)) => {
                Some(at.saturating_duration_since(Instant::now()))
            }
            None => None,
        }
    }

    /// Once per turn, after every store mutation: starts a push when
    /// the state changed since the last placement. Never blocks: the
    /// socket opens non-blocking and the line queues, and the daemon's
    /// poll set drives the rest. Returns a line for the log when the
    /// attempt failed fast with no bar (one line per failed change;
    /// later attempts of the same change stay silent).
    pub fn sync(&mut self, store: &Store) -> Option<String> {
        let current = State::of(store);
        if self.last == Some(current) {
            return None;
        }
        self.last = Some(current);
        self.retries = 0;
        self.start(current, true, false)
    }

    /// Retries when the watch fired and the socket may have appeared:
    /// pushes the current state when a send is owed and nothing is
    /// already scheduled. Drains the watch either way (a
    /// level-triggered fd left unread would spin the poll). Silent: the
    /// change's line already went out (or the change never earned one),
    /// so a re-appearing absence must not chatter under a file flood.
    /// A scheduled re-dial is left alone: the watch fires on any file
    /// creation, and an unrelated one must not eat the spell's retry.
    pub fn retry(&mut self, store: &Store) -> Option<String> {
        let _ = self.socket_may_have_appeared();
        if !self.owed || self.pending.is_some() || self.redial.is_some() {
            return None;
        }
        let current = State::of(store);
        self.last = Some(current);
        self.start(current, false, false)
    }

    /// Advances the feed one turn: drives the in-flight push, fires a
    /// due re-dial, and retries the owed send when the watch fired
    /// (the daemon passes whether it did). Returns a line for the log
    /// when an attempt finished loudly. Cheap at idle (no pending,
    /// nothing owed): a few branches, no I/O, no allocation.
    pub fn service(&mut self, store: &Store, watch_fired: bool) -> Option<String> {
        if watch_fired && let Some(line) = self.retry(store) {
            return Some(line);
        }
        if self.pending.is_none() {
            return self.fire_redial(store);
        }
        self.drive()
    }

    /// Fires a due re-dial while owed: the race grace, or the bounded
    /// retry while the bar stays reachable. Silent either way (the
    /// spell's line went out on its first failure); the retry parks
    /// past the cap, leaving the watch and the next change.
    fn fire_redial(&mut self, store: &Store) -> Option<String> {
        let race = match self.redial.as_ref() {
            None => return None,
            Some(Redial::Race(at)) => {
                if Instant::now() < *at {
                    return None;
                }
                true
            }
            Some(Redial::Retry(at)) => {
                if Instant::now() < *at {
                    return None;
                }
                false
            }
        };
        self.redial = None;
        let current = State::of(store);
        self.last = Some(current);
        if race {
            return self.start(current, true, true);
        }
        if !(self.owed && self.reachable) {
            return None;
        }
        self.retries += 1;
        if self.retries > self.retry_cap {
            // Parked: the watch or the next change retries. Silent:
            // the spell's line went out on its first failure.
            return None;
        }
        self.start(current, false, false)
    }

    /// Starts one push of `current` without blocking: the socket opens
    /// non-blocking, the line queues, and `drive` advances what it can
    /// at once (a healthy bar's connect and write complete here, still
    /// without waiting). `loud` logs this attempt's fast failure;
    /// `race` marks the bind-listen grace re-dial, whose refusal parks
    /// rather than scheduling another.
    fn start(&mut self, current: State, loud: bool, race: bool) -> Option<String> {
        self.pending = None;
        self.redial = None;
        self.reachable = false;
        self.owed = false;
        let socket = self.socket_path()?;
        let line = set_line(&payload(current, &self.icon_unread, &self.icon_dnd));
        let deadline = Instant::now() + self.push_deadline;
        match dial(&socket) {
            Dial::Connected(stream) => {
                self.pending = Some(Pending {
                    stream,
                    line,
                    written: 0,
                    phase: Phase::Write,
                    read: Vec::new(),
                    deadline,
                    loud,
                    race,
                });
            }
            Dial::Connecting(stream) => {
                self.pending = Some(Pending {
                    stream,
                    line,
                    written: 0,
                    phase: Phase::Connect,
                    read: Vec::new(),
                    deadline,
                    loud,
                    race,
                });
            }
            Dial::Absent(detail) => {
                self.owed = true;
                self.arm();
                if loud {
                    return Some(format!(
                        "scootnotify: cannot reach the bar ({detail}); leaving it"
                    ));
                }
                return None;
            }
            Dial::Stale => {
                self.owed = true;
                self.arm();
                // The bar may be mid-bind (the socket file lands before
                // it listens): one silent grace re-dial closes that
                // race, and its outcome is the reported one.
                self.redial = Some(Redial::Race(Instant::now() + RACE_AFTER));
                return None;
            }
        }
        self.drive()
    }

    /// Drives the in-flight push without blocking: connect, write and
    /// read in a loop until one would block, then the deadline.
    /// Returns a line for the log when the attempt finished loudly.
    fn drive(&mut self) -> Option<String> {
        loop {
            match self.step() {
                Step::Wait => return self.check_deadline(),
                Step::More => {}
                Step::Done(outcome) => {
                    if let Some(line) = self.finish(outcome) {
                        return Some(line);
                    }
                    // A phase advanced synchronously
                    // (connect-then-write): keep driving. Every
                    // terminal outcome clears the push above, so its
                    // absence here ends the drive.
                    self.pending.as_ref()?;
                }
            }
        }
    }

    /// One non-blocking step of the in-flight push: `Wait` when it
    /// would block, `More` on forward progress, `Done` when the
    /// attempt finished.
    fn step(&mut self) -> Step {
        let Some(pending) = self.pending.as_mut() else {
            return Step::Wait;
        };
        match pending.phase {
            Phase::Connect => {
                if !out_ready(pending.stream.as_fd()) {
                    return Step::Wait;
                }
                match rustix::net::getpeername(pending.stream.as_fd()) {
                    Ok(_) => {
                        pending.phase = Phase::Write;
                        Step::More
                    }
                    Err(_) => Step::Done(Outcome::Stale),
                }
            }
            Phase::Write => {
                match rustix::io::write(pending.stream.as_fd(), &pending.line[pending.written..]) {
                    Ok(n) => {
                        pending.written += n;
                        if pending.written >= pending.line.len() {
                            pending.phase = Phase::Read;
                        }
                        Step::More
                    }
                    Err(error) if is_would_block(&error) => Step::Wait,
                    Err(error) => Step::Done(Outcome::Unanswered(short_errno(&error))),
                }
            }
            Phase::Read => {
                let mut chunk = [0u8; 1024];
                match rustix::io::read(pending.stream.as_fd(), &mut chunk) {
                    Ok(0) => Step::Done(Outcome::Unanswered(String::from(
                        "the bar closed the connection",
                    ))),
                    Ok(n) => {
                        pending.read.extend_from_slice(&chunk[..n]);
                        if let Some(end) = pending.read.iter().position(|byte| *byte == b'\n') {
                            Step::Done(parse_reply(&pending.read[..end]))
                        } else if pending.read.len() as u64 > MAX_REPLY {
                            Step::Done(Outcome::Refused(String::from(
                                "unreadable reply from the bar",
                            )))
                        } else {
                            Step::More
                        }
                    }
                    Err(error) if is_would_block(&error) => Step::Wait,
                    Err(error) => Step::Done(Outcome::Unanswered(short_errno(&error))),
                }
            }
        }
    }

    /// Reaps an in-flight push past its deadline: the bar stayed
    /// reachable but unanswering.
    fn check_deadline(&mut self) -> Option<String> {
        let past = self
            .pending
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.deadline);
        if !past {
            return None;
        }
        self.finish(Outcome::Unanswered(format!(
            "no reply within {} s",
            self.push_deadline.as_secs()
        )))
    }

    /// Settles one finished push: the placement, the owed flag, the
    /// watch and the re-dial. Returns a line for the log on failure:
    /// first attempts speak (`loud`), scheduled re-dials stay silent,
    /// so one change costs one line however many attempts it takes
    /// (an error reply always speaks: it names the bar's own refusal).
    /// Success is silent. A settled change still counts as placed:
    /// the watch or the next change retries what is owed, never a
    /// turn.
    fn finish(&mut self, outcome: Outcome) -> Option<String> {
        let (loud, race) = self
            .pending
            .as_ref()
            .map(|pending| (pending.loud, pending.race))
            .unwrap_or((false, false));
        match outcome {
            Outcome::Sent => {
                self.pending = None;
                self.owed = false;
                self.redial = None;
                self.reachable = false;
                self.disarm();
                None
            }
            Outcome::NotPlaced => {
                // Retrying the same payload cannot help, and the next
                // change retries (silently). The guidance goes out
                // once per run.
                self.pending = None;
                self.owed = false;
                self.redial = None;
                self.reachable = false;
                self.disarm();
                if self.placed_logged {
                    None
                } else {
                    self.placed_logged = true;
                    Some(guidance())
                }
            }
            Outcome::Refused(detail) => {
                // The bar answered an error that is not a missing
                // module (unreachable with our fixed shape): the socket
                // exists, so no appearance will fire, and the next
                // change retries.
                self.pending = None;
                self.owed = false;
                self.redial = None;
                self.reachable = false;
                self.disarm();
                Some(format!(
                    "scootnotify: cannot reach the bar ({detail}); leaving it"
                ))
            }
            Outcome::Stale => {
                self.pending = None;
                self.owed = true;
                self.reachable = false;
                self.arm();
                if race {
                    // The grace re-dial refused too: a stale socket. It
                    // parks on the watch (a rebind recreates the file).
                    self.redial = None;
                    Some(String::from(
                        "scootnotify: cannot reach the bar (connection refused); leaving it",
                    ))
                } else {
                    self.redial = Some(Redial::Race(Instant::now() + RACE_AFTER));
                    None
                }
            }
            Outcome::Unanswered(detail) => {
                self.pending = None;
                self.owed = true;
                self.reachable = true;
                self.arm();
                // A reachable-but-quiet bar may just be slow: the
                // bounded retry converges without a new change, while
                // the watch covers a rebind.
                self.redial = Some(Redial::Retry(Instant::now() + self.retry_after));
                loud.then(|| {
                    format!("scootnotify: cannot reach the bar ({detail}); retrying while owed")
                })
            }
        }
    }

    /// The bar's socket path: `None` when the environment names none
    /// (no runtime dir, a too-long path), which reads as no bar.
    fn socket_path(&self) -> Option<PathBuf> {
        match &self.target {
            Target::Fixed(path) => Some(path.clone()),
            Target::Env => scootui::control::paths::from_env("scootbar")
                .map(|paths| paths.socket)
                .ok(),
        }
    }

    /// Arms the one-shot watch on the runtime dir for the socket's
    /// appearance. Best effort: no watch means the next change still
    /// retries, only a bar started later waits longer.
    fn arm(&mut self) {
        if self.watch.is_some() {
            return;
        }
        let Some(socket) = self.socket_path() else {
            return;
        };
        let Some(dir) = socket.parent() else {
            return;
        };
        if !dir.is_dir() {
            return;
        }
        let Some(name) = socket
            .file_name()
            .map(|name| name.as_encoded_bytes().to_vec())
        else {
            return;
        };
        use rustix::fs::inotify::{CreateFlags, WatchFlags, add_watch, init};
        let watch = init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .ok()
            .and_then(|fd| {
                add_watch(&fd, dir, WatchFlags::CREATE | WatchFlags::MOVED_TO)
                    .ok()
                    .map(|_| Watch { fd, name })
            });
        self.watch = watch;
    }

    /// Disarms the watch: nothing is owed, so nothing watches.
    fn disarm(&mut self) {
        self.watch = None;
    }

    /// Drains the watch: whether the bar's socket may have appeared
    /// since. An unframed buffer answers `true` (a needless dial costs
    /// less than a missed bar), like the D-Bus link's scan.
    fn socket_may_have_appeared(&mut self) -> bool {
        let Some(watch) = self.watch.as_ref() else {
            return false;
        };
        let mut buf = [0u8; 1024];
        let mut seen = false;
        loop {
            match rustix::io::read(&watch.fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if scan_names(&buf[..n], &watch.name) {
                        seen = true;
                    }
                    // Level-triggered: keep draining while it speaks.
                    continue;
                }
                Err(_) => break,
            }
        }
        seen
    }
}

/// Whether an inotify buffer holds an event for `want`: whole-field
/// matching on the file name. An unframed buffer answers `true`: a
/// needless dial costs less than a missed bar (the D-Bus link's rule).
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
        // The name is NUL-padded in the event: compare without it.
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

/// What one finished push attempt did.
enum Outcome {
    /// The bar answered `ok`.
    Sent,
    /// The bar answered an error naming the unplaced module.
    NotPlaced,
    /// The bar answered another error: settled like a placement (the
    /// next change retries), with no watch to arm.
    Refused(String),
    /// A dial refused: the bind-listen race, or a stale socket.
    Stale,
    /// Past the connect the bar went quiet: a write/read error, a
    /// closed connection, or the deadline.
    Unanswered(String),
}

/// What opening the bar's socket found, without blocking.
enum Dial {
    /// Connected at once (the common path for a healthy bar).
    Connected(UnixStream),
    /// Connecting: `OUT` plus `getpeername` tells.
    Connecting(UnixStream),
    /// No socket (or no path): only an appearance can help.
    Absent(String),
    /// Refused: the bind-listen race, or a stale socket.
    Stale,
}

/// Opens the bar's socket without blocking: the serving path never
/// waits on the bar.
fn dial(socket: &Path) -> Dial {
    use rustix::net::{AddressFamily, SocketAddrAny, SocketAddrUnix, SocketFlags, SocketType};
    let addr = match SocketAddrUnix::new(socket) {
        Ok(addr) => SocketAddrAny::from(addr),
        Err(error) => return Dial::Absent(short_errno(&error)),
    };
    let stream = match rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    ) {
        Ok(fd) => UnixStream::from(fd),
        Err(error) => return Dial::Absent(short_errno(&error)),
    };
    match rustix::net::connect(&stream, &addr) {
        Ok(()) => Dial::Connected(stream),
        Err(error) if error == rustix::io::Errno::INPROGRESS => Dial::Connecting(stream),
        Err(error) if error == rustix::io::Errno::NOENT => Dial::Absent(short_errno(&error)),
        Err(_) => Dial::Stale,
    }
}

/// Whether `fd` is writable now, without blocking: the connect
/// phase's poll.
fn out_ready(fd: BorrowedFd<'_>) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
    let zero = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    match poll(&mut fds, Some(&zero)) {
        Ok(_) => !fds[0].revents().is_empty(),
        Err(_) => false,
    }
}

/// The not-placed guidance: once per run.
fn guidance() -> String {
    String::from(
        "scootnotify: the bar shows no notifications module (add \"notifications\" to a bar list in programs.scootbar.settings); leaving it",
    )
}

/// The request line: the bar's protocol version, `set`, our module.
fn set_line(payload: &serde_json::Value) -> Vec<u8> {
    let mut line = Vec::new();
    line.extend_from_slice(br#"{"protocol":1,"type":"set","id":""#);
    line.extend_from_slice(MODULE_ID.as_bytes());
    line.extend_from_slice(br#"","value":"#);
    // Serializing strings and numbers into a `Vec` cannot fail; if it
    // ever did, the empty value gets a `Malformed` refusal, not a
    // panic, and the send is owed like any other failure.
    if serde_json::to_writer(&mut line, payload).is_err() {
        line.extend_from_slice(b"null");
    }
    line.push(b'}');
    line.push(b'\n');
    line
}

/// Reads the one-line reply: `ok` is sent, an error naming the
/// unplaced module is not-placed, any other error is refused.
fn parse_reply(reply: &[u8]) -> Outcome {
    #[derive(serde::Deserialize)]
    struct Reply {
        #[serde(rename = "type")]
        kind: String,
        message: Option<String>,
    }
    match serde_json::from_slice::<Reply>(reply) {
        Ok(reply) if reply.kind == "ok" => Outcome::Sent,
        Ok(reply) if reply.kind == "error" => {
            let message = reply.message.unwrap_or_default();
            if message.contains("is not placed") {
                Outcome::NotPlaced
            } else {
                Outcome::Refused(message)
            }
        }
        _ => Outcome::Refused(String::from("unreadable reply from the bar")),
    }
}

/// Whether an Errno means "try again on the next turn" rather than a
/// failure: the socket would block, or a signal interrupted the call.
/// Matched through `std`'s kinds, so both spellings of each errno
/// count however rustix names them.
fn is_would_block(error: &rustix::io::Errno) -> bool {
    matches!(
        std::io::Error::from(*error).kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}

/// One line for the log from an Errno: the kind, never a dump.
fn short_errno(error: &rustix::io::Errno) -> String {
    format!("{}", std::io::Error::from(*error))
}

#[cfg(test)]
mod tests;
