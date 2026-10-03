//! The battery module: charge level and state, warn and urgent classes,
//! and a low-battery hook.
//!
//! `/sys/class/power_supply/*` read on change, woken by kernel uevents on
//! a `NETLINK_KOBJECT_UEVENT` socket (group 1). The `power_supply` filter
//! is in userspace: the bar wakes on every kernel uevent and drops the
//! rest. Whether the driver emits uevents as the capacity changes or only
//! on plug and unplug was the ticket's measured question, and it is
//! answered on the Asahi M2 (`macsmc-battery`): plug and unplug each emit
//! a burst of `power_supply` uevents, and capacity steps while discharging
//! emit none (five steps over 62 minutes, zero uevents; AC at Full is
//! silent too). So a uevent is how the module learns it has started
//! discharging, and a slow timer (once a minute) is how it sees the steps:
//! it re-reads while discharging and stops when charging or full, with the
//! rate published ([`DISCHARGE_POLL`], `docs/scootbar/cli.md`). Other
//! drivers may differ; the measurement is in
//! `docs/scootbar/backlog/resolved/battery-unplug-uevent-measure-done.md`.
//!
//! ## States
//!
//! The uevent socket is owned whenever batteries are present and costs one
//! wake per kernel event, however many datagrams arrive: [`Battery::on_ready`]
//! drains them all (bounded per turn) and re-reads sysfs once, so a uevent
//! storm is one re-read per turn. The timerfd exists only while
//! discharging, so a charged machine owns one fd. No battery at all
//! (desktop, VM, an empty `power_supply` directory) is [`Init::Unavailable`]:
//! no fds, no width, no log spam. A battery removed at runtime empties the
//! view (the module hides) and drops the timer; the uevent socket stays as
//! the appearance watch, so a reinsert shows again with no polling.
//!
//! ## Untrusted text, bounded reads
//!
//! Sysfs files are small, so each is read once into a fixed buffer: a
//! longer file cannot grow anything. `capacity` past 100 is clamped (a
//! driver quirk, and 104% is wrong-er than 100%), an unparsable one skips
//! its battery, and a status string no kernel documents maps to `Unknown`
//! rather than refusing the battery. Uevent datagrams are scanned for a
//! whole `SUBSYSTEM=power_supply` field; anything else is drained.
//!
//! ## No allocation past start-up
//!
//! File reads use fixed stack buffers, and the hook's argv is cloned once
//! per low crossing (taken once by the loop). The entry-name list is
//! rebuilt per re-read; re-reads are rare (uevents and the minute timer),
//! never per frame.

use std::fmt::Write;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::event::PollFlags;
use rustix::net::netlink::{KOBJECT_UEVENT, SocketAddrNetlink};
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, socket_with};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use super::{ActionSpec, Init, Module, OutputView, Sources, Update, View};
use crate::action::Action;

#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "battery";

/// The actions a binding may name: none, like the clock. A click or scroll
/// binding runs a configured command; the module defines nothing of its
/// own.
pub const ACTIONS: &[ActionSpec] = &[];

/// The default `warn-below`: the class turns `warn` at or below this percent.
pub const DEFAULT_WARN_BELOW: u32 = 20;
/// The default `urgent-below`: the class turns `urgent` at or below this
/// percent, and the `on-low` hook fires crossing it downward.
pub const DEFAULT_URGENT_BELOW: u32 = 10;
/// The most a threshold takes, in percent.
pub const MAX_THRESHOLD: u32 = 100;

/// How often sysfs is re-read while discharging: the published fallback
/// rate for drivers whose capacity steps emit no uevent. Runs only while
/// discharging; charging, full and absent batteries own no timer.
pub const DISCHARGE_POLL: Duration = Duration::from_secs(60);

/// Datagrams drained per ready turn at most: a uevent storm is a bounded
/// number of reads and one re-read, not an unbounded one. Leftovers stay
/// readable for the next turn.
const MAX_DATAGRAMS_PER_TURN: usize = 64;
/// The most `power_supply` entries scanned per re-read: a real machine has
/// a handful; past this the rest are left out.
const MAX_BATTERIES: usize = 32;
/// Bytes read from one sysfs file at most: every file used fits in a few
/// (`capacity` is digits, `status` a word, `type` a word). A longer file is
/// not a value of ours.
const MAX_FILE: usize = 64;

/// Which batteries the shown level comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Batteries {
    /// The mean capacity, and the merged state (discharging wins: losing
    /// power is what matters).
    #[default]
    Combine,
    /// The first usable battery in sorted name order.
    First,
}

impl Batteries {
    /// The value in the config file.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "combine" => Some(Self::Combine),
            "first" => Some(Self::First),
            _ => None,
        }
    }
}

/// What a battery (or their combination) holds for the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Level {
    percent: u32,
    state: State,
    /// How many batteries went into it: 1 for `First`, the count for
    /// `Combine`, so `query` says which.
    count: u32,
}

/// A battery's state, from its `status` file. What no kernel documents is
/// `Unknown`, never a refusal: drivers invent strings, and a level with an
/// honest unknown state beats no level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    Charging,
    Discharging,
    Full,
    NotCharging,
    #[default]
    Unknown,
}

impl State {
    fn parse(text: &str) -> Self {
        match text {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" => Self::Full,
            "Not charging" => Self::NotCharging,
            _ => Self::Unknown,
        }
    }

    /// The `query` value's word.
    fn name(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Full => "full",
            Self::NotCharging => "not-charging",
            Self::Unknown => "unknown",
        }
    }

    /// The tooltip's word.
    fn word(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::NotCharging => "Not charging",
            Self::Unknown => "Unknown",
        }
    }
}

/// The module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub warn_below: u32,
    pub urgent_below: u32,
    pub batteries: Batteries,
    /// The `on-low` command line, run once per downward crossing of
    /// `urgent-below`.
    pub on_low: Option<Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            warn_below: DEFAULT_WARN_BELOW,
            urgent_below: DEFAULT_URGENT_BELOW,
            batteries: Batteries::Combine,
            on_low: None,
        }
    }
}

pub fn init(settings: &super::Settings) -> Init {
    match start_with(
        &settings.battery,
        Path::new("/sys/class/power_supply"),
        None,
    ) {
        Ok(module) => Init::Available(module),
        Err(why) => Init::Unavailable(why),
    }
}

/// Tests only: the module started as if the probe had found batteries — a
/// fixture `power_supply` directory at `root`, with uevents arriving on
/// `uevent` (a socketpair peer the test writes crafted datagrams to), so
/// the contract test drives it on a machine without a battery.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // One fixture per test thread: `cargo test` shares a process across
    // threads, and nextest does not, so the pid plus the thread id is
    // unique either way. Left behind on purpose (four tiny files): the
    // stand-in cannot hand a guard back through its signature, and a
    // per-run directory is rewritten, not duplicated. (The snapshots keep
    // the same convention under `$TMPDIR/scootbar-snapshots/`.)
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "scootbar-battery-standin-{}-{:?}-{id}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&root);
    // A discharging battery at half charge: every source and view below
    // has something to work with.
    tests::write_fixture(
        &root,
        "BAT0",
        tests::Fixture {
            present: Some("1"),
            capacity: Some("50"),
            status: Some("Discharging"),
        },
    );
    let (peer, ours) = tests::socketpair();
    std::mem::forget(peer);
    start_with(&settings.battery, &root, Some(ours)).expect("the fixture is a battery")
}

/// Starts the module at `root`, with uevents arriving on `uevent` (or a
/// fresh kernel uevent socket when `None`). `Err` says why there is no
/// battery to show.
fn start_with(
    settings: &Settings,
    root: &Path,
    uevent: Option<OwnedFd>,
) -> Result<Box<dyn Module>, String> {
    let uevent = match uevent {
        Some(fd) => fd,
        None => uevent_socket().map_err(|error| format!("no uevent socket: {error}"))?,
    };
    let mut battery = Battery {
        warn_below: settings.warn_below.min(MAX_THRESHOLD),
        urgent_below: settings.urgent_below.min(MAX_THRESHOLD),
        batteries: settings.batteries,
        on_low: settings.on_low.clone(),
        root: root.to_path_buf(),
        uevent,
        tick: None,
        shown: read_level(root, settings.batteries),
        // Starting below the hook's threshold is not a crossing: arm only
        // above it, so a boot at 5% does not fire.
        low_armed: true,
        hook: None,
        said_gone: false,
    };
    battery.low_armed = battery
        .shown
        .is_none_or(|level| level.percent > battery.urgent_below);
    battery.sync_timer();
    let Some(_) = battery.shown else {
        return Err(format!(
            "no battery under {} (a desktop or VM has none)",
            root.display()
        ));
    };
    Ok(Box::new(battery))
}

/// The kernel uevent tap: a datagram socket bound to group 1 (every kernel
/// uevent; the module filters to `power_supply`). Receiving needs no
/// privilege; only the kernel (or a privileged sender) can emit.
fn uevent_socket() -> io::Result<OwnedFd> {
    let fd = socket_with(
        AddressFamily::NETLINK,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        Some(KOBJECT_UEVENT),
    )?;
    bind(&fd, &SocketAddrNetlink::new(0, 1))?;
    Ok(fd)
}

/// The module: what it shows, and the two fds it may own (the uevent tap
/// always, the discharge timer only while discharging).
struct Battery {
    warn_below: u32,
    urgent_below: u32,
    batteries: Batteries,
    on_low: Option<Vec<String>>,
    root: PathBuf,
    uevent: OwnedFd,
    tick: Option<Tick>,
    shown: Option<Level>,
    /// Whether the next downward crossing of `urgent-below` fires the hook:
    /// false after it fired, until the level rises back above.
    low_armed: bool,
    /// The hook's argv, taken once by the loop through
    /// [`Module::take_action`].
    hook: Option<Vec<String>>,
    /// Whether the runtime removal was said already: once per
    /// disappearance, not per wake.
    said_gone: bool,
}

/// The discharge timer: a periodic `timerfd`, read when it fires.
struct Tick {
    fd: OwnedFd,
}

impl Tick {
    fn new() -> io::Result<Self> {
        let fd = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )?;
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: DISCHARGE_POLL.as_secs() as i64,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: DISCHARGE_POLL.as_secs() as i64,
                tv_nsec: 0,
            },
        };
        timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec)?;
        Ok(Self { fd })
    }

    fn read(&self) {
        let mut expirations = [0u8; 8];
        let _ = rustix::io::read(&self.fd, &mut expirations);
    }
}

impl Battery {
    /// Arms or drops the discharge timer to match what is shown: ticking
    /// only while a battery is discharging.
    fn sync_timer(&mut self) {
        let discharging = self
            .shown
            .is_some_and(|level| level.state == State::Discharging);
        if discharging && self.tick.is_none() {
            // No fd to arm (out of descriptors): uevents still wake the
            // module; the timer is the fallback, not the source.
            if let Ok(tick) = Tick::new() {
                self.tick = Some(tick);
            }
        } else if !discharging {
            self.tick = None;
        }
    }

    /// Re-reads sysfs and moves the shown level: the timer follows the
    /// state, the hook follows the threshold, and the return says whether
    /// the view moved.
    fn refresh(&mut self) -> Update {
        let level = read_level(&self.root, self.batteries);
        if level == self.shown {
            return Update::Unchanged;
        }
        self.shown = level;
        self.sync_timer();
        self.track_low();
        if level.is_none() {
            // A runtime removal: one line, then silence until a battery is
            // back (the uevent socket is the appearance watch).
            if !self.said_gone {
                self.said_gone = true;
                crate::print::warn(format_args!(
                    "scootbar: battery: no battery under {} any more; waiting for one",
                    self.root.display()
                ));
            }
        } else {
            self.said_gone = false;
        }
        Update::Changed
    }

    /// Arms or fires the low hook on the shown level: a downward crossing
    /// of `urgent-below` stages the command (taken once by the loop), and
    /// rising back above re-arms it. Clones only on a crossing, never per
    /// re-read.
    fn track_low(&mut self) {
        if self.on_low.is_none() {
            return;
        }
        let Some(level) = self.shown else {
            return;
        };
        if level.percent <= self.urgent_below {
            if self.low_armed {
                self.low_armed = false;
                // One crossing, one run: a hook the loop never takes (no
                // loop in a test) is replaced, not queued.
                self.hook = self.on_low.clone();
            }
        } else {
            self.low_armed = true;
        }
    }

    /// Handles the uevent tap: drains every pending datagram (bounded per
    /// turn; leftovers stay readable) and re-reads once when any of them
    /// is a `power_supply` event. A receive error other than "drained" (the
    /// kernel reports a netlink overflow as `ENOBUFS`) means events were
    /// lost, so it counts as a power change. Anything else is drained
    /// silence: netlink uevent sockets do not disconnect, so there is no
    /// reconnect path to need.
    fn on_uevent(&mut self) -> Update {
        let mut power = false;
        let mut buf = [0u8; 8192];
        for _ in 0..MAX_DATAGRAMS_PER_TURN {
            match rustix::net::recv(&self.uevent, &mut buf, RecvFlags::empty()) {
                Ok((_, n)) if has_power_supply(&buf[..n]) => power = true,
                Ok(_) => {}
                Err(err) => {
                    // Drained (AGAIN) or a loss: either way the loop ends,
                    // and a loss is one more re-read, never a spin.
                    power |= recv_lost_events(err);
                    break;
                }
            }
        }
        if power {
            self.refresh()
        } else {
            Update::Unchanged
        }
    }

    /// The class of the shown level: `urgent` wins over `warn`, by level
    /// alone (a charging battery at 5% is still a battery at 5%).
    fn class(&self) -> super::Class {
        match self.shown {
            Some(level) if level.percent <= self.urgent_below => super::Class::Urgent,
            Some(level) if level.percent <= self.warn_below => super::Class::Warn,
            _ => super::Class::Normal,
        }
    }
}

/// Whether a failed receive means datagrams were lost rather than that the
/// queue is empty: `AGAIN` is empty, anything else (`ENOBUFS` on a netlink
/// overflow) may have dropped a `power_supply` event.
fn recv_lost_events(err: rustix::io::Errno) -> bool {
    err != rustix::io::Errno::AGAIN
}

/// Whether the datagram carries a whole `SUBSYSTEM=power_supply` field:
/// NUL-separated `KEY=VALUE` pairs, matched whole, so a longer value
/// (`power_supply_now`) cannot false-positive.
fn has_power_supply(datagram: &[u8]) -> bool {
    datagram
        .split(|byte| *byte == 0)
        .any(|field| field == b"SUBSYSTEM=power_supply")
}

/// Reads the combined level at `root`: the usable batteries in sorted name
/// order, combined per `batteries`, or `None` where there are none.
fn read_level(root: &Path, batteries: Batteries) -> Option<Level> {
    let entries = sorted_entries(root)?;
    let mut sum = 0u32;
    let mut count = 0u32;
    let mut first: Option<Level> = None;
    let mut merged = State::Unknown;
    for name in entries.iter().take(MAX_BATTERIES) {
        let Some(level) = read_one(root, name) else {
            continue;
        };
        if first.is_none() {
            first = Some(Level { count: 1, ..level });
        }
        sum += level.percent;
        count += 1;
        merged = merge(merged, level.state);
    }
    match batteries {
        Batteries::First => first,
        Batteries::Combine => sum.checked_div(count).map(|percent| Level {
            percent,
            state: merged,
            count,
        }),
    }
}

/// Merges one battery's state into the combination: discharging wins (power
/// going out is what matters), then charging, then not-charging; full only
/// when everything is full, unknown contributing nothing.
fn merge(into: State, state: State) -> State {
    match (into, state) {
        (State::Discharging, _) | (_, State::Discharging) => State::Discharging,
        (State::Charging, _) | (_, State::Charging) => State::Charging,
        (State::NotCharging, _) | (_, State::NotCharging) => State::NotCharging,
        (State::Full, State::Full) => State::Full,
        (State::Full, State::Unknown) => State::Full,
        (State::Unknown, other) => other,
    }
}

/// The usable batteries' names in sorted order, or `None` where the
/// directory cannot be listed at all. One bad entry skips itself, never
/// the whole directory.
fn sorted_entries(root: &Path) -> Option<Vec<String>> {
    let read = std::fs::read_dir(root).ok()?;
    let mut names = Vec::new();
    for entry in read {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        names.push(name.to_owned());
    }
    names.sort();
    Some(names)
}

/// One battery's level, or `None` where the entry is not a usable battery:
/// the wrong type, an empty slot (`present` 0), an unreadable or
/// unparsable capacity, or a missing file. Allocates nothing: fixed buffers
/// on the stack.
fn read_one(root: &Path, name: &str) -> Option<Level> {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return None;
    }
    let dir = root.join(name);
    let kind_path = dir.join("type");
    let present_path = dir.join("present");
    let capacity_path = dir.join("capacity");
    let status_path = dir.join("status");
    if read_small(&kind_path).is_none_or(|kind| kind.text() != "Battery") {
        return None;
    }
    // An empty slot (`present` 0) is not a battery. Missing the file, some
    // drivers do not have it, counts as present.
    if let Some(present) = read_small(&present_path) {
        if present.text() != "1" {
            return None;
        }
    }
    let capacity = read_small(&capacity_path)?;
    let percent = parse_capacity(capacity.text())?;
    let state = read_small(&status_path)
        .map(|status| State::parse(status.text()))
        .unwrap_or(State::Unknown);
    Some(Level {
        // Above 100 is a driver quirk; 104% on the bar is wrong-er than
        // a clamp.
        percent: percent.min(100),
        state,
        count: 1,
    })
}

/// One sysfs file's contents, read once into a fixed buffer: what fits in
/// [`MAX_FILE`] bytes, or nothing when the file cannot be read, is not
/// UTF-8, or is longer. Owned, so callers hold it past the read.
struct Small {
    buf: [u8; MAX_FILE],
    len: usize,
}

impl Small {
    /// The file trimmed of ASCII whitespace: what `echo` and the kernel
    /// leave behind (a trailing newline).
    fn text(&self) -> &str {
        let text = std::str::from_utf8(&self.buf[..self.len]).unwrap_or("");
        trim(text)
    }
}

/// The whole file, or `None` where it cannot be read, is not UTF-8, or is
/// longer than [`MAX_FILE`] bytes. One short read: sysfs files are tiny,
/// and a longer one is not a value of ours.
fn read_small(path: &Path) -> Option<Small> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; MAX_FILE + 1];
    let n = file.read(&mut buf).ok()?;
    if n > MAX_FILE {
        return None;
    }
    if std::str::from_utf8(&buf[..n]).is_err() {
        return None;
    }
    let mut small = Small {
        buf: [0; MAX_FILE],
        len: n,
    };
    small.buf[..n].copy_from_slice(&buf[..n]);
    Some(small)
}

/// ASCII whitespace off both ends: what `echo` and the kernel leave behind
/// (a trailing newline).
fn trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_ascii_whitespace())
}

/// A capacity file's digits, or `None` for anything else. Empty, signed,
/// or trailing-junk values are a driver's problem, not a percent.
fn parse_capacity(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

impl Module for Battery {
    /// The uevent tap always, and the discharge timer while discharging:
    /// two sources at most, zero cost otherwise.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        sources.add(self.uevent.as_fd(), PollFlags::IN);
        if let Some(tick) = &self.tick {
            sources.add(tick.fd.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let _ = events;
        // Source 0 is the uevent tap; source 1, when polled, is the
        // discharge timer. A source the loop hands over is one the module
        // added this turn (the harness holds it to that), so anything else
        // is unreachable; treat it as the tap rather than panic the bar.
        if source == 0 {
            self.on_uevent()
        } else {
            if let Some(tick) = &self.tick {
                tick.read();
            }
            self.refresh()
        }
    }

    /// `72%`, in the level's class; nothing where there is no battery, so
    /// the module hides.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(level) = self.shown else {
            return;
        };
        let _ = write!(view.text_mut(), "{}%", level.percent);
        let _ = write!(
            view.tooltip_mut(),
            "{} {}%",
            level.state.word(),
            level.percent
        );
        view.set_class(self.class());
    }

    /// What `query` reports: the percent, the state and how many batteries
    /// went into it, or nothing where there is nothing shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let level = self.shown?;
        Some(serde_json::json!({
            "percent": level.percent,
            "state": level.state.name(),
            "batteries": level.count,
        }))
    }

    /// The low hook's command, staged by a downward crossing, taken once:
    /// the loop carries it out like a binding.
    fn take_action(&mut self) -> Option<Action> {
        self.hook.take().map(Action::Exec)
    }
}
