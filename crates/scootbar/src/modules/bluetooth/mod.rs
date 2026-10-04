//! The bluetooth module: adapter power and connected devices over BlueZ.
//!
//! A client of `org.bluez` on the system bus, through the shared D-Bus
//! client. It shows the first connected device's name (and its charge,
//! when BlueZ reports one), `on` while an adapter is powered with nothing
//! connected, `off` while every adapter is off, and nothing while there
//! is no adapter or no BlueZ at all. A click toggles the first adapter's
//! power; the `menu` action opens the configured device picker.
//! Nothing is shown while BlueZ is absent, and nothing is polled: the
//! owner coming and going is `NameOwnerChanged`, hotplug is
//! `InterfacesAdded` and `InterfacesRemoved`, and a power, connection,
//! name or charge change is `PropertiesChanged`, all filtered by the bus
//! to `/org/bluez`, so an idle bar with Bluetooth off, without an
//! adapter, without BlueZ, or with devices just connected is woken by
//! nothing (`session.rs`).
//!
//! ## Which device
//!
//! Several may be connected. The one shown is the first in path order, so
//! the choice is the same every run; the tooltip and `query` list them
//! all. The toggle goes to the first adapter in path order.
//!
//! ## States, like the media module's
//!
//! `Waiting` owns an inotify fd on the bus socket's directory and shows
//! nothing; `Live` owns the connection. A dead connection drops back to
//! waiting with nothing shown and dials once more at once, a bus that
//! keeps dropping the bar is left alone for a while, and a machine with no
//! system bus costs one descriptor (`crate::dbus::link`, on the system
//! bus through [`Link::start_on`](crate::dbus::link::Link::start_on)).
//!
//! ## Untrusted bytes
//!
//! Anything on the system bus can own `org.bluez` when BlueZ itself is
//! absent and say anything; it cannot crash, hang or grow the bar, and
//! can only fill the adapter and device slots (`session.rs` says exactly
//! what it can and cannot do). Names are cleaned and cut when stored (a
//! kilobyte name costs a bounded walk), and a run of connects and
//! disconnects is drawn ten times a second at most ([`DRAW_GAP`]), so a
//! device flapping constantly costs a timer, not a redraw each. The
//! battery charge is shown only when BlueZ reports it (`Battery1`); it is
//! never polled.

use std::fmt::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::event::PollFlags;
use rustix::io::Errno;
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};

use super::{
    ActionSpec, ArgKind, Class, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{Action, ModuleAction, Trigger};
use crate::dbus::conn;
use crate::dbus::link::{Addr, Link};
use crate::icon::Icon;

mod session;
mod timer;

use session::Live;
use timer::OneShot;

#[cfg(test)]
mod daemon_tests;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "bluetooth";

/// The actions a binding or an agent may name. None takes a number.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "toggle",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "menu",
        arg: ArgKind::None,
    },
];

/// The most adapters held: a machine has one or two. Past this a
/// newcomer is ignored, said once.
pub const MAX_ADAPTERS: usize = 8;
/// The most devices held: BlueZ remembers many. Past this a newcomer is
/// ignored, said once.
pub const MAX_DEVICES: usize = 64;
/// A call unanswered this long is forgotten when its slot is wanted (no
/// bus times a call out by default: see `Conn::expire`). Short in tests.
#[cfg(not(test))]
const FLIGHT_TTL: Duration = Duration::from_secs(30);
#[cfg(test)]
const FLIGHT_TTL: Duration = Duration::from_millis(600);
/// An object is read (`GetAll`) no oftener than this.
const MIN_REFRESH_GAP: Duration = Duration::from_millis(50);

/// A change of what is shown (a power, a connection, another device), in
/// a run of them, is drawn at most this often: the first after a quiet
/// spell at once, the rest held for one timer, the latest shown when it
/// fires. A device flapping between connected and disconnected hundreds
/// of times a second costs the bar ten redraws a second, not hundreds.
/// The module appearing or emptying (a first adapter, the last gone, the
/// bus lost) is never held.
const DRAW_GAP: Duration = Duration::from_millis(100);

/// The module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The picker: spawned with the device list on stdin. Empty runs
    /// nothing, and the action is refused saying so.
    pub menu_command: Vec<String>,
    /// A static icon, when the config sets the icon keys: shown in every
    /// state for which no per-state icon is set.
    pub icon: Option<Icon>,
    /// One glyph per state, when the config sets the per-state keys:
    /// every adapter off, one powered with nothing connected, and a
    /// device connected. Each wins over the static icon for its own
    /// state.
    pub icon_off: Option<Icon>,
    pub icon_on: Option<Icon>,
    pub icon_connected: Option<Icon>,
    /// Whether the text is drawn beside the icon. `false` draws only the
    /// icon, with the text moved into the tooltip (which already names
    /// the state: `Bluetooth off`, `Bluetooth on`, the device list).
    pub show_text: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            menu_command: Vec::new(),
            icon: None,
            icon_off: None,
            icon_on: None,
            icon_connected: None,
            show_text: true,
        }
    }
}

pub fn init(settings: &super::Settings) -> Init {
    Init::Available(start_with(
        &settings.bluetooth,
        Addr::Path(conn::system_bus_path()),
    ))
}

/// Tests only: the module started on a scripted bus (a socketpair whose
/// far end answers the set-up and then holds still), so the contract test
/// drives the connected path on a machine without any bus.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    use std::io::Read;
    let (ours, mut theirs) = std::os::unix::net::UnixStream::pair().expect("a socketpair");
    // Parked on the far end for the life of the process: the contract
    // calls this once.
    std::thread::spawn(move || {
        crate::dbus::testdaemon::serve_setup(&mut theirs);
        let _ = theirs.set_read_timeout(None);
        let mut sink = [0u8; 4096];
        while theirs.read(&mut sink).is_ok_and(|n| n > 0) {}
    });
    start_with(&settings.bluetooth, Addr::Stream(ours))
}

fn start_with(settings: &Settings, addr: Addr) -> Box<dyn Module> {
    Box::new(Bluetooth {
        link: Link::start_on("bluetooth", "system bus", addr, session::start),
        menu_command: settings.menu_command.clone(),
        icon: settings.icon.clone(),
        icon_off: settings.icon_off.clone(),
        icon_on: settings.icon_on.clone(),
        icon_connected: settings.icon_connected.clone(),
        show_text: settings.show_text,
        menu: None,
        drawn: None,
        held: None,
    })
}

/// A running device picker, for the reap.
struct Menu {
    child: Child,
    pidfd: Option<OwnedFd>,
}

/// The module.
struct Bluetooth {
    link: Link<Live>,
    menu_command: Vec<String>,
    icon: Option<Icon>,
    icon_off: Option<Icon>,
    icon_on: Option<Icon>,
    icon_connected: Option<Icon>,
    show_text: bool,
    menu: Option<Menu>,
    /// When a change was last reported to the bar.
    drawn: Option<Instant>,
    /// Armed while a change waits out [`DRAW_GAP`]: the view the bar
    /// asks for when it fires is the latest, whatever came meanwhile.
    held: Option<OneShot>,
}

/// What the view is drawn from, compared before and after each turn so
/// the bar redraws on a real change and nothing else: whether anything is
/// shown, and the revision of what it says.
type Shown = Option<(bool, u64)>;

impl Bluetooth {
    fn shown(&self) -> Shown {
        let live = self.link.live()?;
        if live.adapters.is_empty() {
            return None;
        }
        Some((true, live.rev))
    }

    /// Reports a change to the bar now.
    fn draw_now(&mut self) -> Update {
        self.held = None;
        self.drawn = Some(Instant::now());
        Update::Changed
    }

    /// Reports a change that is not the module appearing or emptying: at
    /// once after a quiet spell, else held for one timer ([`DRAW_GAP`]
    /// since the last), however many follow.
    fn changed(&mut self) -> Update {
        if self.held.is_some() {
            return Update::Unchanged;
        }
        let wait = self
            .drawn
            .map(|at| DRAW_GAP.saturating_sub(at.elapsed()))
            .filter(|wait| !wait.is_zero());
        match wait.and_then(OneShot::after) {
            Some(timer) => {
                self.held = Some(timer);
                Update::Unchanged
            }
            // Past the gap, or no timer to wait with: now.
            None => self.draw_now(),
        }
    }

    /// What a turn did to the view, from what was shown `before`: the bar
    /// is told at once when the module appears or empties (a first
    /// adapter, the last gone, the bus goes: `dropped`), and any other
    /// change goes through [`Bluetooth::changed`]'s gap.
    fn after(&mut self, before: Shown, dropped: bool) -> Update {
        let now = self.shown();
        if dropped || before.is_none() != now.is_none() {
            self.draw_now()
        } else if before != now {
            self.changed()
        } else {
            Update::Unchanged
        }
    }

    /// Whether any adapter is powered.
    fn any_powered(&self) -> bool {
        self.link
            .live()
            .is_some_and(|live| live.adapters.iter().any(|a| a.powered))
    }

    /// The device shown: the first connected one in path order.
    fn shown_device(&self) -> Option<(String, Option<u8>)> {
        let live = self.link.live()?;
        let device = live
            .devices
            .iter()
            .filter(|d| d.connected)
            .min_by(|a, b| a.path.cmp(&b.path))?;
        let battery = if device.has_battery {
            device.battery
        } else {
            None
        };
        Some((device.name.clone(), battery))
    }

    /// The connected devices in path order, for the tooltip, the query
    /// and the picker.
    fn connected(&self) -> Vec<(String, Option<u8>)> {
        let Some(live) = self.link.live() else {
            return Vec::new();
        };
        let mut devices: Vec<_> = live
            .devices
            .iter()
            .filter(|d| d.connected)
            .map(|d| {
                (
                    d.path.clone(),
                    d.name.clone(),
                    if d.has_battery { d.battery } else { None },
                )
            })
            .collect();
        devices.sort_by(|a, b| a.0.cmp(&b.0));
        devices
            .into_iter()
            .map(|(_, name, battery)| (name, battery))
            .collect()
    }

    /// Reaps the picker when it exited: by pidfd where there is one, by
    /// `try_wait` everywhere. Returns whether a picker is still running.
    fn reap_menu(&mut self) -> bool {
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        if let Some(pidfd) = menu.pidfd.as_ref() {
            let mut expirations = [0u8; 8];
            match rustix::io::read(pidfd, &mut expirations) {
                Ok(_) => {}
                Err(Errno::AGAIN) => {
                    let _ = menu.child.try_wait();
                    return true;
                }
                Err(_) => {}
            }
        }
        match menu.child.try_wait() {
            Ok(Some(_)) | Err(_) => {
                self.menu = None;
                false
            }
            Ok(None) => true,
        }
    }

    /// Opens the picker: spawns `menu-command` with the device list on
    /// stdin. Fire and forget (the command owns the choice; the bar never
    /// reads it back), reaped by pidfd when it exits. Idempotent while
    /// one is already open.
    fn open_menu(&mut self) -> Result<Update, InvokeError> {
        if self.menu.is_some() {
            return Ok(Update::Unchanged);
        }
        if self.menu_command.is_empty() {
            return Err(InvokeError::Refused("no menu command configured"));
        }
        let Some(live) = self.link.live() else {
            return Err(InvokeError::Refused("no system bus"));
        };
        if live.adapters.is_empty() {
            return Err(InvokeError::Refused("no adapter"));
        }
        let mut list = Vec::new();
        let mut devices: Vec<_> = live.devices.iter().collect();
        devices.sort_by(|a, b| a.path.cmp(&b.path));
        for device in devices {
            let mut line = device.name.clone();
            if device.connected {
                line.push_str(" (connected)");
            }
            line.push('\n');
            list.extend_from_slice(line.as_bytes());
        }
        if list.is_empty() {
            return Err(InvokeError::Refused("no devices seen yet"));
        }
        let (read, write) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .map_err(|_| InvokeError::Refused("cannot start the menu command"))?;
        let mut command = Command::new(&self.menu_command[0]);
        command.args(&self.menu_command[1..]);
        command.stdin(Stdio::from(read));
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::inherit());
        let child = command
            .spawn()
            .map_err(|_| InvokeError::Refused("cannot start the menu command"))?;
        // The list is a few kilobytes against a 64 KiB pipe: one write
        // carries it, even when the menu never reads. A short write still
        // leaves a usable prefix; the menu, not the bar, owns the choice.
        let mut input: std::fs::File = write.into();
        use std::io::Write as _;
        let _ = input.write_all(&list);
        drop(input);
        let pidfd = rustix::process::pidfd_open(
            rustix::process::Pid::from_child(&child),
            rustix::process::PidfdFlags::empty(),
        )
        .ok();
        // Without pidfds (pre-5.3 kernels) the child is reaped
        // opportunistically on every ready turn instead.
        self.menu = Some(Menu { child, pidfd });
        self.reap_menu();
        Ok(Update::Unchanged)
    }
}

impl Module for Bluetooth {
    /// The link's fds (the bus socket, or the directory watch), then the
    /// refresh timer while an object waits out its gap, then the draw
    /// timer while a change of text is held, then the picker's pidfd
    /// while one runs. No other timer, ever: an idle module wakes
    /// nothing.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        self.link.watch(&mut |fd, events| {
            sources.add(fd, events);
        });
        if let Some(timer) = self.link.live().and_then(|live| live.coalesce.as_ref()) {
            sources.add(timer.as_fd(), PollFlags::IN);
        }
        if let Some(timer) = &self.held {
            sources.add(timer.as_fd(), PollFlags::IN);
        }
        if let Some(pidfd) = self.menu.as_ref().and_then(|menu| menu.pidfd.as_ref()) {
            sources.add(pidfd.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let own = self.link.source_count();
        if source >= own {
            // Past the link's sources: the refresh timer first while it
            // is armed, then the draw timer, then the picker's pidfd.
            let mut timer = source - own;
            let refreshing = self.link.live().is_some_and(|live| live.coalesce.is_some());
            if refreshing && timer == 0 {
                let before = self.shown();
                if let Some(live) = self.link.live_mut() {
                    live.on_coalesce();
                }
                return self.after(before, false);
            }
            if refreshing {
                timer -= 1;
            }
            let holding = self.held.is_some();
            if holding && timer == 0 {
                if let Some(held) = self.held.take() {
                    held.drain();
                    return self.draw_now();
                }
            }
            if holding {
                timer -= 1;
            }
            if timer == 0 {
                self.reap_menu();
            }
            return Update::Unchanged;
        }
        let before = self.shown();
        let dropped = self.link.on_ready(source, events, &mut |live, event| {
            live.apply(event);
            false
        });
        self.reap_menu();
        self.after(before, dropped)
    }

    /// What is shown: the first connected device's name (with its charge
    /// when BlueZ reports one) while an adapter is powered, `on` while
    /// one is powered with nothing connected, `off` while every adapter
    /// is off (adapter power dominates a device that still claims to be
    /// connected: its disconnect is on its way), and nothing while there
    /// is no adapter at all, so the module hides. With an icon the glyph
    /// (or picture) stands before the text, or alone with `show-text =
    /// false` (the tooltip already names the state, so the text is moved
    /// into it).
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(live) = self.link.live() else {
            return;
        };
        if live.adapters.is_empty() {
            return;
        }
        if !self.any_powered() {
            if self.show_text {
                let _ = write!(view.text_mut(), "off");
            }
            view.set_class(Class::Muted);
            let _ = write!(view.tooltip_mut(), "Bluetooth off");
            if let Some(icon) = self.icon_off.clone().or_else(|| self.icon.clone()) {
                view.show_icon(&icon);
            }
        } else if let Some((name, battery)) = self.shown_device() {
            if self.show_text {
                let _ = write!(view.text_mut(), "{name}");
                if let Some(percentage) = battery {
                    let _ = write!(view.text_mut(), " {percentage}%");
                }
            }
            let connected = self.connected();
            let _ = write!(view.tooltip_mut(), "Bluetooth: ");
            for (at, (name, battery)) in connected.iter().enumerate() {
                if at > 0 {
                    let _ = write!(view.tooltip_mut(), ", ");
                }
                let _ = write!(view.tooltip_mut(), "{name}");
                if let Some(percentage) = battery {
                    let _ = write!(view.tooltip_mut(), " {percentage}%");
                }
            }
            if let Some(icon) = self.icon_connected.clone().or_else(|| self.icon.clone()) {
                view.show_icon(&icon);
            }
        } else {
            if self.show_text {
                let _ = write!(view.text_mut(), "on");
            }
            let _ = write!(view.tooltip_mut(), "Bluetooth on");
            if let Some(icon) = self.icon_on.clone().or_else(|| self.icon.clone()) {
                view.show_icon(&icon);
            }
        }
    }

    /// What `query` reports: the state (`off`, `on` or `connected`), the
    /// adapters and the connected devices, or nothing while there is no
    /// adapter.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let live = self.link.live()?;
        if live.adapters.is_empty() {
            return None;
        }
        let connected = self.connected();
        let state = if !self.any_powered() {
            "off"
        } else if connected.is_empty() {
            "on"
        } else {
            "connected"
        };
        let mut value = serde_json::json!({
            "state": state,
            "adapters": live.adapters.len(),
            "powered": self.any_powered(),
            "connected": connected.len(),
        });
        if let Some((name, battery)) = self.shown_device() {
            value["device"] = serde_json::Value::String(name);
            if let Some(percentage) = battery {
                value["battery"] = serde_json::Value::from(percentage);
            }
        }
        Some(value)
    }

    /// A click toggles the first adapter's power, with no binding at
    /// all; nothing while there is no adapter (nothing to act on, and no
    /// warning for a click on an empty module).
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        let Trigger::Click = input.trigger else {
            return None;
        };
        let live = self.link.live()?;
        if live.adapters.is_empty() {
            return None;
        }
        Some(Action::Module(ModuleAction::new("toggle", None)))
    }

    /// Carries out `toggle` and `menu`: one `Set` for the first adapter's
    /// power (never blocking the bar and never waiting for the answer:
    /// the state changes when BlueZ says so, as a signal), and the
    /// picker. A refusal says why.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = steps;
        if action.arg.is_some() {
            return Err(InvokeError::NoArg);
        }
        match &*action.name {
            "toggle" => {
                let Some(live) = self.link.live_mut() else {
                    return Err(InvokeError::Refused("no system bus"));
                };
                // The first adapter's own power: with one adapter (the
                // machine with two is rare) this is the adapter's toggle,
                // and with several it is still one click, one adapter, one
                // signal.
                let Some(first) = live.adapters.iter().min_by(|a, b| a.path.cmp(&b.path)) else {
                    return Err(InvokeError::Refused("no adapter"));
                };
                let powered = !first.powered;
                live.set_powered(powered);
                Ok(Update::Unchanged)
            }
            "menu" => self.open_menu(),
            _ => Err(InvokeError::Unknown),
        }
    }

    /// Its view carries a tooltip: the state and the connected devices.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// A click toggles power with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }
}
