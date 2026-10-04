//! The brightness module: the panel backlight's level, scroll to adjust.
//!
//! `/sys/class/backlight/*/brightness` against `max_brightness`, read on
//! change, woken by kernel uevents on a `NETLINK_KOBJECT_UEVENT` socket
//! filtered to the `backlight` subsystem — the same transport as the
//! battery module, which proved the pattern (one tap, whole-field filter,
//! bounded drain, one re-read per turn). The ticket's measurement stands:
//! on the Asahi M2 writing `apple-panel-bl/brightness` emits exactly one
//! `change` uevent per write, with no value in its properties, so every
//! uevent re-reads `brightness`. `actual_brightness` can differ from it by
//! 1 (rounding); what is shown is the set point, what was written.
//!
//! ## States
//!
//! The uevent socket is owned whenever a backlight is present and costs one
//! wake per kernel event, however many datagrams arrive:
//! [`Brightness::on_ready`] drains them all (bounded per turn) and re-reads
//! sysfs once, so a uevent storm is one re-read per turn. There is no
//! timer, ever: the fallback the ticket allowed (re-read only when the bar
//! itself changed the level) needs none, because a write here is a
//! synchronous syscall, answered before `invoke` returns — the view is
//! re-read right after the write, and the uevent the kernel then emits
//! finds nothing new. No backlight at all (desktop, VM, an empty
//! `backlight` directory) is [`Init::Unavailable`]: no fds, no width, no
//! log spam. A backlight removed at runtime empties the view (the module
//! hides) and keeps the socket as the appearance watch, so a reinsert
//! shows again with no polling.
//!
//! ## Writes
//!
//! Setting the level is a plain write of the raw value to the device's
//! `brightness` file: no daemon, no child, no D-Bus (logind's
//! `SetBrightness` waits for M6's shared client; until then a direct write
//! is what a permitted bar does). It needs permission — a udev rule for
//! the backlight class or the `video` group grants it — and where the bar
//! has none the action is refused naming that, not retried and not
//! silenced. Every write is an absolute raw value from the shown percent
//! (never an accumulated relative one), so a touchpad flood cannot drift;
//! there is nothing in flight to coalesce, because each write completes
//! before the next begins. A write never lands below raw 1: 0 blanks the
//! panel on the drivers measured, and a bar must not darken its own screen
//! past what a scroll can bring back.
//!
//! ## Untrusted text, bounded reads
//!
//! Sysfs files are small, so each is read once into a fixed buffer: a
//! longer file cannot grow anything. An unparsable or missing `brightness`
//! or `max_brightness`, a zero `max_brightness`, or a device name that
//! could escape the class directory skips its device. Uevent datagrams are
//! scanned for a whole `SUBSYSTEM=backlight` field; anything else is
//! drained.
//!
//! ## No allocation past start-up
//!
//! File reads use fixed stack buffers, and the written raw value is
//! decimal-encoded into one too. The device-name list is rebuilt per
//! re-read; re-reads are rare (uevents only), never per frame.

use std::fmt::Write;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::event::PollFlags;
use rustix::net::netlink::{KOBJECT_UEVENT, SocketAddrNetlink};
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, socket_with};

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};
use crate::icon::{Art, Icon};

#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "brightness";

/// The actions a binding or an agent may name: `raise` and `lower` move by
/// the config's step (a scroll's steps arrive through the scroll itself,
/// one step each), and `set` takes the absolute percent.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "raise",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "lower",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "set",
        arg: ArgKind::Required,
    },
];

/// The default `step`: percent points per scroll notch and per raise.
pub const DEFAULT_STEP: u32 = 5;
/// The most `step` takes, in percent points.
pub const MAX_STEP: u32 = 50;

/// Datagrams drained per ready turn at most: a uevent storm is a bounded
/// number of reads and one re-read, not an unbounded one. Leftovers stay
/// readable for the next turn.
const MAX_DATAGRAMS_PER_TURN: usize = 64;
/// The most `backlight` entries scanned per probe: a real machine has one
/// or two; past this the rest are left out.
const MAX_DEVICES: usize = 32;
/// Bytes read from one sysfs file at most: every file used holds digits.
/// A longer file is not a value of ours.
const MAX_FILE: usize = 64;
/// Digits of a raw backlight value at most (`u32::MAX` is ten).
const MAX_RAW_DIGITS: usize = 10;

/// The class directory, unless a test points elsewhere.
const CLASS: &str = "/sys/class/backlight";

/// What the module shows: the level as a percent, and the raw values it
/// came from (a write maps the next percent back through the same range).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Level {
    percent: u32,
    device: String,
    raw: u32,
    max: u32,
}

/// The module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The backlight to follow, by class-directory name. None is the first
    /// usable device in sorted name order.
    pub device: Option<String>,
    pub step: u32,
    /// The brightness icon from `icon`: one glyph shown at every level,
    /// or one glyph per level (dim to bright), picked by [`level_for`];
    /// or a static path or picture.
    pub icon: Option<BrightnessIcon>,
    /// Whether the text is drawn beside the icon. `false` draws only the
    /// icon, with the text moved into the tooltip (which already names
    /// the device and the level).
    pub show_text: bool,
}

/// The brightness icon from `icon`: one glyph shown at every level, or
/// one glyph per level (dim to bright), picked by [`level_for`]. A
/// static `icon-path`/`icon-image` icon has no levels: per-level vector
/// or PNG icons are out of scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrightnessIcon {
    /// One glyph for every level.
    One(char),
    /// One glyph per level, dim to bright.
    Levels([char; 4]),
    /// A static path or picture, shown at every level.
    Art(Art),
}

/// The icon level for `percent`: 0 (dim) to 3 (bright), in quartiles
/// (0–24, 25–49, 50–74, 75–100).
fn level_for(percent: u32) -> usize {
    (percent.min(100) / 25).min(3) as usize
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: None,
            step: DEFAULT_STEP,
            icon: None,
            show_text: true,
        }
    }
}

#[cfg(feature = "brightness")]
pub fn init(settings: &super::Settings) -> Init {
    match start_with(&settings.brightness, Path::new(CLASS), None) {
        Ok(module) => Init::Available(module),
        Err(why) => Init::Unavailable(why),
    }
}

/// Tests only: the module started as if the probe had found a backlight —
/// a fixture class directory at `root`, with uevents arriving on `uevent`
/// (a socketpair peer the test writes crafted datagrams to), so the
/// contract test drives it on a machine without one.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // One fixture per test thread: `cargo test` shares a process across
    // threads, and nextest does not, so the pid plus the thread id is
    // unique either way. Left behind on purpose (a few tiny files): the
    // stand-in cannot hand a guard back through its signature, and a
    // per-run directory is rewritten, not duplicated. (The snapshots keep
    // the same convention under `$TMPDIR/scootbar-snapshots/`.)
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "scootbar-brightness-standin-{}-{:?}-{id}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&root);
    // A panel at almost half: every source and view below has something
    // to work with.
    tests::write_fixture(
        &root,
        "apple-panel-bl",
        tests::Fixture { raw: Some("251") },
        509,
    );
    let (peer, ours) = tests::socketpair();
    std::mem::forget(peer);
    start_with(&settings.brightness, &root, Some(ours)).expect("the fixture is a backlight")
}

/// Starts the module at `root`, with uevents arriving on `uevent` (or a
/// fresh kernel uevent socket when `None`). `Err` says why there is no
/// backlight to show.
fn start_with(
    settings: &Settings,
    root: &Path,
    uevent: Option<OwnedFd>,
) -> Result<Box<dyn Module>, String> {
    let uevent = match uevent {
        Some(fd) => fd,
        None => uevent_socket().map_err(|error| format!("no uevent socket: {error}"))?,
    };
    let step = settings.step.clamp(1, MAX_STEP);
    let shown = read_level(root, settings.device.as_deref());
    let Some(level) = shown else {
        return Err(match &settings.device {
            Some(device) => format!("no backlight `{device}` under {}", root.display()),
            None => format!(
                "no backlight under {} (a desktop or VM has none)",
                root.display()
            ),
        });
    };
    Ok(Box::new(Brightness {
        step,
        device: level.device.clone(),
        root: root.to_path_buf(),
        uevent,
        shown: Some(level),
        icon: settings.icon.clone(),
        show_text: settings.show_text,
        said_gone: false,
    }))
}

/// The kernel uevent tap: a datagram socket bound to group 1 (every kernel
/// uevent; the module filters to `backlight`). Receiving needs no
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

/// The module: what it shows, and the one fd it owns.
struct Brightness {
    step: u32,
    /// The device followed: the config's, or the first usable one found.
    device: String,
    root: PathBuf,
    uevent: OwnedFd,
    shown: Option<Level>,
    icon: Option<BrightnessIcon>,
    show_text: bool,
    /// Whether the runtime removal was said already: once per
    /// disappearance, not per wake.
    said_gone: bool,
}

impl Brightness {
    /// Re-reads sysfs and moves the shown level; the return says whether
    /// the view moved.
    fn refresh(&mut self) -> Update {
        let level = read_level(&self.root, Some(&self.device));
        if level == self.shown {
            return Update::Unchanged;
        }
        self.shown = level;
        if self.shown.is_none() {
            // A runtime removal: one line, then silence until the device
            // is back (the uevent socket is the appearance watch).
            if !self.said_gone {
                self.said_gone = true;
                crate::print::warn(format_args!(
                    "scootbar: brightness: no backlight `{}` under {} any more; waiting for one",
                    self.device,
                    self.root.display()
                ));
            }
        } else {
            self.said_gone = false;
        }
        Update::Changed
    }

    /// Handles the uevent tap: drains every pending datagram (bounded per
    /// turn; leftovers stay readable) and re-reads once when any of them
    /// is a `backlight` event. Anything else, and a read error, is
    /// drained silence: netlink uevent sockets do not disconnect, so there
    /// is no reconnect path to need.
    fn on_uevent(&mut self) -> Update {
        let mut backlight = false;
        let mut buf = [0u8; 8192];
        for _ in 0..MAX_DATAGRAMS_PER_TURN {
            match rustix::net::recv(&self.uevent, &mut buf, RecvFlags::empty()) {
                Ok((_, n)) if has_backlight(&buf[..n]) => backlight = true,
                Ok(_) => {}
                Err(rustix::io::Errno::AGAIN) => break,
                Err(_) => break,
            }
        }
        if backlight {
            self.refresh()
        } else {
            Update::Unchanged
        }
    }

    /// The icon for `percent`: the level's glyph, or the static glyph,
    /// path or picture. `None` where no icon is set: without any icon
    /// the level shows text alone, as before.
    fn icon_for(&self, percent: u32) -> Option<Icon> {
        match &self.icon {
            Some(BrightnessIcon::One(glyph)) => Some(Icon::Glyph(*glyph)),
            Some(BrightnessIcon::Levels(levels)) => Some(Icon::Glyph(levels[level_for(percent)])),
            Some(BrightnessIcon::Art(art)) => Some(Icon::Art(art.clone())),
            None => None,
        }
    }

    /// Writes an absolute percent: clamped to the range, and never below
    /// the raw floor that keeps the panel lit. Re-reads right after, so
    /// what is shown is what the file holds; the uevent the write emits
    /// then finds nothing new.
    fn write_percent(&mut self, percent: u32) -> Result<Update, InvokeError> {
        let shown = self
            .shown
            .as_ref()
            .ok_or(InvokeError::Refused("no backlight to change"))?;
        let raw = percent_to_raw(percent, shown.max);
        write_raw(&self.root, &self.device, raw).map_err(|_| {
            InvokeError::Refused(
                "the backlight is not writable (a udev rule for the backlight class \
                 or the video group grants writes)",
            )
        })?;
        Ok(self.refresh())
    }
}

/// Whether the datagram carries a whole `SUBSYSTEM=backlight` field:
/// NUL-separated `KEY=VALUE` pairs, matched whole, so a longer value
/// cannot false-positive.
fn has_backlight(datagram: &[u8]) -> bool {
    datagram
        .split(|byte| *byte == 0)
        .any(|field| field == b"SUBSYSTEM=backlight")
}

/// Reads the shown level at `root`: the config's device when one is
/// pinned, else the first usable device in sorted name order — or `None`
/// where there is none.
fn read_level(root: &Path, device: Option<&str>) -> Option<Level> {
    match device {
        Some(name) => read_one(root, name),
        None => {
            let entries = sorted_entries(root)?;
            entries
                .iter()
                .take(MAX_DEVICES)
                .find_map(|name| read_one(root, name))
        }
    }
}

/// One backlight's level, or `None` where the entry is not a usable
/// backlight: a name that could escape the class directory, a missing or
/// unparsable file, or a zero range. Allocates only the device name it
/// keeps.
fn read_one(root: &Path, name: &str) -> Option<Level> {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return None;
    }
    let dir = root.join(name);
    let raw = read_small(&dir.join("brightness"))?;
    let max = read_small(&dir.join("max_brightness"))?;
    let raw = parse_raw(raw.text())?;
    let max = parse_raw(max.text())?;
    if max == 0 {
        return None;
    }
    // Past the range is a driver quirk; showing past 100% is wrong-er
    // than a clamp.
    let raw = raw.min(max);
    Some(Level {
        percent: raw_to_percent(raw, max),
        device: name.to_owned(),
        raw,
        max,
    })
}

/// The usable device names in sorted order, or `None` where the directory
/// cannot be listed at all. One bad entry skips itself, never the whole
/// directory.
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

/// A sysfs value's digits, or `None` for anything else. Empty, signed, or
/// trailing-junk values are a driver's problem, not a level.
fn parse_raw(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

/// A raw value as a percent of its range, rounded half up. In `u64`: a
/// `u32` range times 100 still fits, but there is no reason to prove that
/// at every call site.
fn raw_to_percent(raw: u32, max: u32) -> u32 {
    let raw = u64::from(raw);
    let max = u64::from(max);
    ((raw * 100 + max / 2) / max) as u32
}

/// An absolute percent as the raw value to write: rounded half up,
/// clamped to the range, and never 0 — 0 blanks the panel on the drivers
/// measured, and a bar must not darken its own screen past what a scroll
/// can bring back.
fn percent_to_raw(percent: u32, max: u32) -> u32 {
    let percent = u64::from(percent.min(100));
    let max = u64::from(max);
    ((percent * max + 50) / 100).clamp(1, max) as u32
}

/// Writes a raw value to the device's `brightness` file: the decimal
/// digits into a stack buffer, one write. No allocation, whatever the
/// value.
fn write_raw(root: &Path, device: &str, raw: u32) -> io::Result<()> {
    use std::io::Write;
    let mut digits = [b'0'; MAX_RAW_DIGITS];
    let mut value = raw;
    let mut end = digits.len();
    loop {
        end -= 1;
        digits[end] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut file = std::fs::File::create(root.join(device).join("brightness"))?;
    file.write_all(&digits[end..])
}

impl Module for Brightness {
    /// Its view carries a tooltip.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// The one fd owned: the uevent tap. No timer, ever.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        sources.add(self.uevent.as_fd(), PollFlags::IN);
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let _ = events;
        // Source 0 is the uevent tap: the only source the module adds.
        // A source the loop hands over is one the module added this turn
        // (the harness holds it to that), so anything else is
        // unreachable; treat it as the tap rather than panic the bar.
        let _ = source;
        self.on_uevent()
    }

    /// `{percent}%`, with the device in the tooltip; nothing where there
    /// is no backlight, so the module hides. With an icon the glyph (or
    /// picture) stands before the text, or alone with `show-text =
    /// false` (the tooltip already names the device and the level).
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(level) = self.shown.as_ref() else {
            return;
        };
        if self.show_text {
            let _ = write!(view.text_mut(), "{}%", level.percent);
        }
        let _ = write!(view.tooltip_mut(), "{}: {}%", level.device, level.percent);
        if let Some(icon) = self.icon_for(level.percent) {
            view.show_icon(&icon);
        }
    }

    /// What `query` reports: the percent and the device it is, or nothing
    /// where there is nothing shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let level = self.shown.as_ref()?;
        Some(serde_json::json!({
            "percent": level.percent,
            "device": level.device,
        }))
    }

    /// A scroll raises or lowers; anything else means nothing (a click
    /// with no binding does nothing: there is nothing to toggle).
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        let name = match input.trigger {
            Trigger::ScrollUp => "raise",
            Trigger::ScrollDown => "lower",
            _ => return None,
        };
        Some(crate::action::Action::Module(ModuleAction::new(name, None)))
    }

    /// Carries out `raise`, `lower` and `set` as absolute writes from the
    /// shown percent, clamped to the range. What the write did moves the
    /// view within the call: the file is re-read before returning.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        match &*action.name {
            "raise" | "lower" => {
                if action.arg.is_some() {
                    return Err(InvokeError::NoArg);
                }
            }
            "set" => {
                if action.arg.is_none() {
                    return Err(InvokeError::NeedsArg);
                }
            }
            _ => return Err(InvokeError::Unknown),
        }
        let base = self
            .shown
            .as_ref()
            .map(|level| level.percent)
            .ok_or(InvokeError::Refused("no backlight to change"))?;
        let target = match &*action.name {
            "raise" => base
                .saturating_add(steps.saturating_mul(self.step))
                .min(100),
            "lower" => base.saturating_sub(steps.saturating_mul(self.step)),
            _ => action.arg.unwrap_or(0).clamp(0, 100) as u32,
        };
        // At the limit a scroll changes nothing: not a write. (A percent
        // holds several raw values on a wide range, so this compares in
        // percent: the view is what the scroll moves.)
        if target == base {
            return Ok(Update::Unchanged);
        }
        self.write_percent(target)
    }

    /// A scroll adjusts with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }
}
