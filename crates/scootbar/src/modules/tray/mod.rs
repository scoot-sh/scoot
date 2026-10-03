//! The tray module: the StatusNotifierItem watcher and host, over the
//! shared D-Bus client.
//!
//! The watcher is core infrastructure of the bar, not just a module: the
//! bar owns the `org.kde.StatusNotifierWatcher` name (and its
//! `org.freedesktop` twin), re-acquires it if it is lost, answers item
//! registrations itself, and picks up items that registered before the
//! bar started (the bus is enumerated at connect). Where another process
//! already owns the name, the bar runs as a host against it instead —
//! items still appear, clicks still work — and takes over when the owner
//! leaves. Item menus (`ContextMenu`, the DBusMenu protocol) are not built:
//! [popups](../../../../docs/scootbar/backlog/resolved/popups-done.md) exist
//! to draw one in, but the DBusMenu client that reads a layout does not,
//! so the `menu` action is refused saying that, loudly, not silently.
//!
//! ## States
//!
//! `Waiting` owns an inotify fd on the bus socket's directory and shows
//! nothing: no bus, no items, one watch. `Live` owns the bus socket and
//! shows what registered, in `service + path` order (the KDE watcher's
//! id format). A dead connection drops back to waiting with nothing
//! shown; a bus that never exists costs one watch fd at most, and none
//! where even the watch cannot be armed (a reload starts over, as the
//! volume module's does).
//!
//! ## Events, all asynchronous past connect
//!
//! The set-up (auth, `Hello`, `RequestName`, match rules, enumeration)
//! is a handful of blocking round trips on a local socket, like the
//! volume module's handshake. Past that nothing blocks: `GetAll` answers
//! arrive as pending-call replies on a later turn, `NewIcon` and friends
//! re-read the item then, and `NameOwnerChanged` (tracked centrally, one
//! map for items and watcher names alike) drops vanished items at once —
//! an app that crashes without unregistering disappears with its owner.
//!
//! ## Untrusted bytes, bounded icons
//!
//! Item strings are sanitized once, at parse time (controls stripped,
//! cut like view text). Pixmaps arrive as raw `ARGB32` over the bus:
//! entries past 64 pixels a side are left for the icon cache to scale
//! (at most one kept), at most eight entries an item, each converted to
//! premultiplied once per icon version and drawn from the shared icon
//! cache at the output's real size — icons at exact device pixels,
//! without a per-frame allocation. A malformed or hostile item loses
//! itself (its reply is dropped, its entry skipped), never the bar.

use std::fmt::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rustix::event::PollFlags;
use rustix::fs::inotify::{CreateFlags, WatchFlags};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};
use crate::dbus::conn;
use crate::dbus::proto::Writer;
use crate::icon::Art;
use crate::text::Text;

mod item;
mod watcher;

#[cfg(test)]
use item::{Item, Status, fill};
use watcher::{Live, Mode, setup};

#[cfg(test)]
mod daemon_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod fake;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "tray";

/// The actions a binding or an agent may name. Every one takes the item
/// index (`activate 0`); a click, middle click or scroll supplies its own.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "activate",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "secondary",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "wheel-up",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "wheel-down",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu",
        arg: ArgKind::Required,
    },
];

/// The watcher's names, object and interfaces: the KDE one apps speak,
/// and the freedesktop twin the spec names. Both are owned when free;
/// the KDE one decides the mode.
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_KDE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_FDO: &str = "org.freedesktop.StatusNotifierWatcher";
/// The item interfaces, KDE first (what items send), then the twin.
const ITEM_KDE: &str = "org.kde.StatusNotifierItem";
const ITEM_FDO: &str = "org.freedesktop.StatusNotifierItem";
/// The item properties interface.
const ITEM_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The default item path, for a registration naming a service only.
const ITEM_DEFAULT_PATH: &str = "/StatusNotifierItem";
/// What the watcher reports as its protocol version (KDE answers 0).
const PROTOCOL_VERSION: u32 = 0;

/// The most items shown: past this a registration is answered and then
/// ignored, said once per newcomer on stderr, never polled again.
pub const MAX_ITEMS: usize = 32;
/// The most items one service may hold of those: a buggy app registering
/// path after path loses its own extras, not everyone else's slots.
const MAX_PER_SERVICE: usize = 8;
/// A call unanswered this long is forgotten when its slot is wanted
/// (no bus times a call out by default: a stock dbus-daemon session.conf
/// and dbus-broker were both measured at minutes without one, so an item
/// that never answers `GetAll` would hold a slot for good).
const FLIGHT_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// The longest side of a stored pixmap entry, in pixels: entries past it
/// are left for the icon cache to scale from the single smallest kept.
const MAX_STORED_SIDE: u32 = 64;
/// The most pixmap entries converted per item: an animation past this
/// still shows its first entries.
const MAX_STORED_ICONS: usize = 8;
/// Item strings kept this long at most, in bytes: titles and tooltip
/// text are cut like view text, once, at parse time.
const MAX_ITEM_TEXT: usize = 128;
/// An item is read (`GetAll`) no oftener than this: one that announces a
/// change as fast as it is read (a buggy app, a hostile one) costs 20
/// round trips and redraws a second, not a thousand.
const MIN_REFRESH_GAP: std::time::Duration = std::time::Duration::from_millis(50);
/// How many [`conn::Conn::pump`]s one wake of the bus socket makes (64
/// events each): a flood is worked this much a wake, not in one go.
const MAX_PUMPS_PER_WAKE: usize = 4;
/// A connection that dies sooner than this after it was made is a quick
/// death; [`MAX_QUICK_DEATHS`] in a row and the bar stops redialling.
const QUICK_DEATH: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_QUICK_DEATHS: u8 = 3;
/// After [`MAX_QUICK_DEATHS`] the bar tries once more this long on (and
/// again, each time one try dies quick): short in tests.
#[cfg(not(test))]
const RETRY_AFTER_QUICK_DEATHS: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const RETRY_AFTER_QUICK_DEATHS: std::time::Duration = std::time::Duration::from_millis(2000);
/// A scroll's delta past this magnitude is clamped to it: a touchpad
/// flood is one bounded call, never an accumulated one.
const MAX_SCROLL_DELTA: u32 = 64;

/// The module's options: none of its own yet, so the table holds the
/// margin and the interaction keys like every module's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {}

#[cfg(feature = "tray")]
pub fn init(settings: &super::Settings) -> Init {
    let _ = settings;
    let addr = match conn::bus_path() {
        Ok(path) => BusAddr::Path(path),
        Err(()) => {
            crate::print::warn(format_args!(
                "scootbar: tray: DBUS_SESSION_BUS_ADDRESS names no filesystem path \
                 (abstract and other transports are not dialled); no tray"
            ));
            BusAddr::Unusable
        }
    };
    Init::Available(start_with(addr))
}

/// Tests only: the module started as if the probe had found a bus — a
/// scripted one on a socketpair, so the contract test drives the connected
/// path on a machine without any bus. The fake's handle is leaked on
/// purpose (a thread and a socketpair per stand-in): the signature
/// cannot hand a guard back, and the contract calls it once.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    let _ = settings;
    let (stream, fake) = fake::Fake::pair();
    std::mem::forget(fake);
    start_connected(stream)
}

/// How to reach the bus: the filesystem path, or an already-open stream
/// (the tests' socketpair end).
#[derive(Debug)]
enum BusAddr {
    Path(PathBuf),
    /// The session bus's address names nothing this client dials: the
    /// module is there, shows nothing, and never connects.
    Unusable,
    #[cfg(test)]
    Stream(std::os::unix::net::UnixStream),
}

/// Starts the module: connects now when the bus is there, or waits on it
/// when it is not. Always available: a bus may appear at any time, and
/// waiting costs one inotify fd at most.
fn start_with(addr: BusAddr) -> Box<dyn Module> {
    let (path, stream) = match addr {
        BusAddr::Path(path) => (Some(path), None),
        BusAddr::Unusable => (None, None),
        #[cfg(test)]
        BusAddr::Stream(stream) => (conn::bus_path().ok(), Some(stream)),
    };
    let dial = path.is_some();
    let path = path.unwrap_or_default();
    let mut tray = Tray {
        path,
        bus: Bus::Waiting {
            notify: None,
            dir: PathBuf::new(),
            retry: None,
        },
        since: None,
        quick_deaths: 0,
    };
    match stream {
        Some(stream) => tray.connected(stream),
        // Dial now when the bus is there (a local socket: `Hello` is two
        // round trips); waiting costs nothing when it is not.
        None if dial => {
            tray.connect();
        }
        None => {}
    }
    Box::new(tray)
}

/// Starts the module on an already-open stream: the fake bus's end.
#[cfg(test)]
fn start_connected(stream: std::os::unix::net::UnixStream) -> Box<dyn Module> {
    start_with(BusAddr::Stream(stream))
}

/// The module: the bus path, and the wait or the connection.
struct Tray {
    path: PathBuf,
    bus: Bus,
    /// When the live connection was made: a bus that kills it at once is
    /// told from one that went away.
    since: Option<Instant>,
    /// Connections in a row that died within [`QUICK_DEATH`].
    quick_deaths: u8,
}

enum Bus {
    Waiting {
        notify: Option<Notify>,
        dir: PathBuf,
        /// A one-shot timer, armed only after the bus kept dropping us
        /// ([`MAX_QUICK_DEATHS`]): one more try after [`RETRY_AFTER_QUICK_DEATHS`],
        /// so a bus that dislikes one item's answer does not turn the tray
        /// off for the session.
        retry: Option<OwnedFd>,
    },
    Live(Box<Live>),
}

/// The inotify watch on the bus socket's directory, while waiting.
struct Notify {
    fd: OwnedFd,
}

impl Tray {
    /// Waits on the bus socket's directory: the socket's own directory
    /// when there is one, else the runtime directory itself. A watch that
    /// cannot be armed is nothing polled: the socket is re-probed on
    /// every wake that does arrive, and a reload starts over (the volume
    /// module's rule).
    fn wait(&mut self) {
        let dir = self
            .path
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(runtime_dir);
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
                .map(|_| Notify { fd })
            });
        self.bus = Bus::Waiting {
            notify,
            dir,
            retry: None,
        };
    }

    /// Drops the connection with nothing shown (a dead bus's last icons
    /// are not icons), then dials once more at once: a transient death
    /// with a live bus heals in the same turn instead of sticking on a
    /// watch that never fires, and a bus truly gone falls back to
    /// waiting. One attempt only, so a refusing bus cannot hot-loop the
    /// bar: the next try waits on the socket's directory like the first.
    fn drop_live(&mut self) -> Update {
        let had = match &self.bus {
            Bus::Live(live) => !live.items.is_empty(),
            Bus::Waiting { .. } => false,
        };
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
            // A bus that takes us in and drops us at once, again and again
            // (it dislikes something we send): dialling forever would spin
            // the bar. Wait for the socket to be made anew instead.
            if self.quick_deaths == MAX_QUICK_DEATHS {
                self.quick_deaths = self.quick_deaths.saturating_add(1);
                crate::print::warn(format_args!(
                    "scootbar: tray: the bus keeps dropping the connection; \
                     trying again in {} s, or when it is restarted",
                    RETRY_AFTER_QUICK_DEATHS.as_secs()
                ));
            }
            self.arm_retry();
        } else {
            self.connect();
        }
        if had {
            Update::Changed
        } else {
            Update::Unchanged
        }
    }

    /// Connects a stream that is already open (the tests' socketpair end,
    /// or a dialled bus): `Hello` is the only blocking call, and the
    /// names, matches and enumeration follow ready-driven.
    fn connected(&mut self, stream: std::os::unix::net::UnixStream) {
        match conn::setup(stream) {
            Ok(conn) => {
                self.since = Some(Instant::now());
                self.bus = Bus::Live(Box::new(setup(conn)));
            }
            Err(_) => self.wait(),
        }
    }

    /// Dials the bus and starts the set-up; failures wait, said on
    /// stderr so an empty tray names its reason. Nothing is shown until
    /// the names answer.
    fn connect(&mut self) {
        match conn::connect(&self.path) {
            Ok(conn) => {
                self.since = Some(Instant::now());
                self.bus = Bus::Live(Box::new(setup(conn)));
            }
            Err(error) => {
                crate::print::warn(format_args!(
                    "scootbar: tray: no session bus ({error}); waiting for one"
                ));
                self.wait();
            }
        }
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
    /// bar back to waiting (the count is left one short of the limit).
    fn on_retry(&mut self) -> Update {
        if let Bus::Waiting { retry, .. } = &mut self.bus {
            if let Some(timer) = retry.take() {
                let mut expirations = [0u8; 8];
                let _ = rustix::io::read(&timer, &mut expirations);
            }
        }
        self.quick_deaths = MAX_QUICK_DEATHS - 1;
        self.connect();
        Update::Unchanged
    }

    /// Handles the directory watch: drains it (else it stays ready) and
    /// probes the socket when its name arrived. Events for other names
    /// cost one scan of the read bytes, never a connect.
    fn on_notify(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            self.wait();
            return Update::Unchanged;
        }
        let mut bus = false;
        let Bus::Waiting { notify, dir, .. } = &mut self.bus else {
            return Update::Unchanged;
        };
        if let Some(notify) = notify.as_ref() {
            let fd = notify.fd.as_fd();
            let mut buf = [0u8; 4096];
            loop {
                match rustix::io::read(fd, &mut buf) {
                    Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
                    Ok(n) => {
                        if scan_names(&buf[..n], bus_name(&self.path)) {
                            bus = true;
                        }
                    }
                    Err(_) => {
                        self.wait();
                        return Update::Unchanged;
                    }
                }
            }
        }
        let _ = dir;
        if bus {
            // The socket was made anew: whatever dropped us before is
            // over, so the count starts again.
            self.quick_deaths = 0;
            self.connect();
        }
        Update::Unchanged
    }

    /// One turn on the bus: pumps the connection and works each event,
    /// re-pumping while capped so a burst drains in its turn.
    fn on_bus(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return self.drop_live();
        }
        let Bus::Live(live) = &mut self.bus else {
            return Update::Unchanged;
        };
        if live.conn.dead() {
            return self.drop_live();
        }
        let mut changed = Update::Unchanged;
        // A bounded number of pumps a wake: a flood backs up in the socket
        // and is worked a wake at a time (the poll returns at once while
        // it is readable), so the bar's other sources get their turn.
        for _ in 0..MAX_PUMPS_PER_WAKE {
            let (events, capped) = live.conn.pump();
            for event in events {
                if live.apply(event) == Update::Changed {
                    changed = Update::Changed;
                }
                if live.conn.dead() {
                    return self.drop_live();
                }
            }
            if !capped {
                break;
            }
        }
        changed
    }

    /// The item index `x` (device pixels from the span's left, `padding`
    /// inside) lands on at `em`, or `None` in a gap or past the end.
    fn hit(x: u32, padding: u32, em: f32, count: usize) -> Option<usize> {
        let side = Text::art_side(em) as usize;
        let gap = gap(side);
        let at = (x as usize).saturating_sub(padding as usize);
        let stride = side + gap;
        if stride == 0 {
            return None;
        }
        let index = at / stride;
        if index < count && at % stride < side {
            Some(index)
        } else {
            None
        }
    }
}

/// The gap between neighbouring icons: an eighth of the slot, at least
/// one device pixel.
fn gap(side: usize) -> usize {
    (side / 8).max(1)
}

/// The bus socket's file name, for the watch scan.
fn bus_name(path: &Path) -> &[u8] {
    path.file_name()
        .map(|name| name.as_encoded_bytes())
        .unwrap_or(b"bus")
}

/// The runtime directory, or its conventional fallback.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()))
}

/// Whether an inotify buffer holds an event for `want`: whole-field
/// matching on the file name, so a longer name cannot false-positive (the
/// volume module's scan).
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
        // The name is NUL-terminated in the event: compare without it.
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

impl Module for Tray {
    /// Its view carries a tooltip.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// The bus socket while live (with `OUT` while answers wait), then
    /// the directory watch while waiting. No timer, ever: every refresh
    /// is bus-driven, and the wait is an inotify watch.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        match &self.bus {
            Bus::Live(live) => {
                let mut flags = PollFlags::IN;
                // Writable also while staged messages wait their turn: the
                // poll returns at once and the work goes on, a wake at a time.
                if live.conn.want_write() || live.conn.has_staged_work() {
                    flags |= PollFlags::OUT;
                }
                sources.add(live.conn.as_fd(), flags);
                // Source 1, only while an item waits out the refresh gap.
                if let Some(timer) = &live.coalesce {
                    sources.add(timer.as_fd(), PollFlags::IN);
                }
            }
            Bus::Waiting { notify, retry, .. } => {
                if let Some(notify) = notify {
                    sources.add(notify.fd.as_fd(), PollFlags::IN);
                }
                if let Some(retry) = retry {
                    sources.add(retry.as_fd(), PollFlags::IN);
                }
            }
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        match &mut self.bus {
            Bus::Live(live) => {
                // Source 0 is the bus socket, source 1 (while one waits) the
                // refresh timer. Anything else is unreachable (the harness
                // holds it to that); drained as the socket rather than
                // panicking the bar.
                if source == 1 && live.coalesce.is_some() {
                    live.on_coalesce();
                    return Update::Unchanged;
                }
                self.on_bus(events)
            }
            Bus::Waiting { notify, retry, .. } => {
                // The directory watch is source 0 when there is one, then
                // the retry timer.
                let retry_source = usize::from(notify.is_some());
                if retry.is_some() && source == retry_source {
                    return self.on_retry();
                }
                self.on_notify(events)
            }
        }
    }

    /// Nothing textual: the icons are drawn by [`Module::custom_draw`]
    /// and sized by [`Module::span_extra`], so an empty view with icons
    /// still takes no text. The tooltip lists the shown titles that fit.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Bus::Live(live) = &self.bus else {
            return;
        };
        for item in live.items.iter().filter(|item| item.shown()) {
            let title = if item.title.is_empty() {
                &item.tooltip_title
            } else {
                &item.title
            };
            if title.is_empty() {
                continue;
            }
            let sep = if view.tooltip().is_empty() { "" } else { ", " };
            if view.tooltip().len() + sep.len() + title.len() > super::MAX_TEXT {
                break;
            }
            let _ = write!(view.tooltip_mut(), "{sep}{title}");
        }
    }

    /// The icons' width past the (empty) text: one slot a shown item, and
    /// a gap between neighbours. Zero with no items, so the module hides
    /// like any empty one.
    fn span_extra(&self, measure: &super::Measure<'_>) -> u32 {
        let Bus::Live(live) = &self.bus else {
            return 0;
        };
        let count = live.shown_count();
        if count == 0 {
            return 0;
        }
        let side = Text::art_side(measure.em) as usize;
        (count * side + (count - 1) * gap(side)) as u32
    }

    /// Draws each item's icon at the output's device size, from the
    /// shared icon cache (a `NewIcon` misses once; steady frames hit).
    /// `true`: the loop draws nothing more for this span.
    fn custom_draw(&self, ctx: &mut super::CustomDraw<'_, '_>) -> bool {
        let Some(live) = self.bus_live() else {
            return false;
        };
        if live.shown_count() == 0 {
            return false;
        }
        let side = Text::art_side(ctx.em);
        let stride = side as usize + gap(side as usize);
        let top = (i64::from(ctx.canvas.height()) - i64::from(side)) / 2;
        let mut x = i64::from(ctx.span.x) + i64::from(ctx.padding);
        for item in live.items.iter().filter(|item| item.shown()) {
            if let Some(icon) = item.icon_for(side) {
                let art = Art::Tray(icon.clone());
                if let Some(crate::icon::Bitmap::Premultiplied(pixels)) =
                    ctx.text.bitmap(&art, side)
                {
                    let pixels: &[u8] = pixels;
                    let edge = side as usize;
                    for (gy, row) in pixels.chunks_exact(edge * 4).enumerate() {
                        for (gx, pixel) in row.chunks_exact(4).enumerate() {
                            let pixel = [pixel[0], pixel[1], pixel[2], pixel[3]];
                            ctx.canvas.blend_premultiplied(
                                x + gx as i64,
                                top + gy as i64,
                                pixel,
                                ctx.span,
                            );
                        }
                    }
                }
            }
            x += stride as i64;
        }
        true
    }

    /// A click activates, a middle click secondarily, a scroll scrolls —
    /// each on the item under the pointer, with no binding at all. A
    /// right click means nothing by default: the menu is not built (no DBusMenu client),
    /// and silence beats a refusal on every click.
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        let live = self.bus_live()?;
        let slot = Self::hit(
            input.at.x,
            input.at.padding,
            input.at.em,
            live.shown_count(),
        )?;
        let index = live.nth_shown(slot)?;
        let name = match input.trigger {
            Trigger::Click => "activate",
            Trigger::MiddleClick => "secondary",
            Trigger::ScrollUp => "wheel-up",
            Trigger::ScrollDown => "wheel-down",
            Trigger::RightClick => return None,
        };
        Some(crate::action::Action::Module(ModuleAction::new(
            name,
            Some(index as i32),
        )))
    }

    /// Carries out the item actions: `activate`, `secondary` and the two
    /// scrolls call the item (never blocking the bar); `menu` is refused
    /// saying menus are not built. Every one takes the item index.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let index = action.arg.ok_or(InvokeError::NeedsArg)?;
        let name = match &*action.name {
            "activate" | "secondary" | "wheel-up" | "wheel-down" | "menu" => &*action.name,
            _ => return Err(InvokeError::Unknown),
        };
        let Bus::Live(live) = &mut self.bus else {
            return Err(InvokeError::Refused("no bus to call on"));
        };
        let Some(item) = live.at(index) else {
            return Err(InvokeError::Refused("no such tray item"));
        };
        let (service, path) = (item.service.clone(), item.path.clone());
        let mut body = Writer::new();
        let (member, signature) = match name {
            "activate" | "secondary" => {
                body.i32(0);
                body.i32(0);
                if name == "activate" {
                    ("Activate", "ii")
                } else {
                    ("SecondaryActivate", "ii")
                }
            }
            "wheel-up" | "wheel-down" => {
                if steps == 0 {
                    return Ok(Update::Unchanged);
                }
                // The step count, clamped: a touchpad flood is one bounded
                // call. Up is positive, down negative, as the spec's
                // `Scroll(delta, orientation)`.
                let delta = steps.min(MAX_SCROLL_DELTA) as i32;
                body.i32(if name == "wheel-up" { delta } else { -delta });
                body.str("vertical");
                ("Scroll", "is")
            }
            _ => {
                return Err(InvokeError::Refused(
                    "tray menus are not built: no DBusMenu client yet",
                ));
            }
        };
        let Some(bytes) = body.take_body() else {
            return Err(InvokeError::Refused("the call does not fit"));
        };
        live.fire(&service, &path, ITEM_KDE, member, signature, &bytes);
        Ok(Update::Unchanged)
    }

    /// A click activates with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// What `query` reports: the mode and the shown items, or nothing
    /// while nothing is shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let live = self.bus_live()?;
        if live.items.is_empty() {
            return None;
        }
        let mode = match live.mode {
            Mode::Owner => "owner",
            Mode::Host => "host",
        };
        Some(serde_json::json!({
            "watcher": mode,
            "items": live.items.iter().map(|item| serde_json::json!({
                "id": item.id,
                "title": item.title,
                "status": item.status.name(),
                "shown": item.shown(),
            })).collect::<Vec<_>>(),
        }))
    }
}

impl Tray {
    /// The live bus, if there is one.
    fn bus_live(&self) -> Option<&Live> {
        match &self.bus {
            Bus::Live(live) => Some(live),
            Bus::Waiting { .. } => None,
        }
    }
}
