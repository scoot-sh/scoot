//! The power module: lock, log out, suspend, reboot and shut down from
//! a popup menu with a confirm step.
//!
//! One icon (the docs' example is MDI power, U+F0425) and no text: a click
//! bound to `popup` (`on-click = "popup"`, as the volume slider's) opens
//! a popup with one row per action.
//! The first click on a destructive row arms only that row (its label
//! becomes "...? Click again"); a second click on the same row within
//! [`CONFIRM_WINDOW`] performs it and closes the popup. Clicking another
//! row or waiting out the window disarms. The arm survives popup refills
//! and reopens (an armed label is longer, so arming resizes the popup,
//! which reopens it): a close does not disarm at once, but reopening
//! within the window shows the armed row with its explicit confirm label
//! — never a hidden trap — and the window bounds any staleness. Lock
//! needs no confirm.
//!
//! What each row does (every row overridable with a `*-command` argv list,
//! never through a shell, and hideable with `rows`):
//!
//! - **Lock**: `lock-command`. No default (there is no universal locker:
//!   guessing `swaylock` on a GNOME box would fail, or run the wrong
//!   locker), hidden unless configured.
//! - **Log out**: `logout-command` when set, else scoot's `quit` request
//!   over its control socket (the button modules' path). Without a command
//!   and with scoot unreachable the row is absent from the menu and an
//!   invoke of it is refused aloud, rather than flapping with the socket.
//! - **Suspend / Reboot / Shut down**: `suspend-command` and friends when
//!   set, else logind over the system bus (`Suspend`/`Reboot`/`PowerOff`
//!   with `interactive: true`, so polkit decides). A row whose
//!   `CanSuspend`/`CanReboot`/`CanPowerOff` answers `no` or `na` is hidden
//!   (asked when the popup opens — answers older than a minute are
//!   re-asked — not per frame or per refill); anything else — `yes`,
//!   `challenge` (the action call drives authentication), unknown, or no
//!   bus yet — shows it, and a refused call is kept as the last error
//!   (said on stderr, shown in the tooltip and `query`), never silent.
//!
//! An agent's `invoke power logout` (and friends) follows the same
//! two-step as a pointer click: the first arms, the second within the
//! window performs. A single invoke never ends the session. (The direct
//! path for an agent that means it stays `scoot msg action quit`, which
//! is already one explicit call with no confirm.)
//!
//! Idle cost: nothing while closed. The system-bus connection is made when
//! the popup first opens (or an invoke first needs logind), never at
//! start: a bar whose menu is never opened holds no bus fd and makes no
//! round trips. Afterwards one connection stays (like the bluetooth
//! module's), woken only by its own replies.

use std::fmt::Write;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Duration;

use rustix::event::PollFlags;

use super::{ActionSpec, ArgKind, Init, InvokeError, Module, OutputView, Sources, Update, View};
use crate::action::{Action, ModuleAction};
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
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "power";

/// The actions a binding or an agent may name: one per row, plus the
/// popup itself (`on-click = "popup"`). None takes a number.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "lock",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "logout",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "suspend",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "reboot",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "poweroff",
        arg: ArgKind::None,
    },
    #[cfg(feature = "popup")]
    ActionSpec {
        name: crate::action::POPUP,
        arg: ArgKind::None,
    },
];

/// A call unanswered this long is forgotten when its slot is wanted (no
/// bus times a call out by default: see `Conn::expire`). Short in tests.
#[cfg(not(test))]
const FLIGHT_TTL: Duration = Duration::from_secs(30);
#[cfg(test)]
const FLIGHT_TTL: Duration = Duration::from_millis(600);

/// The confirm window: a second click on the same row within this long
/// performs it. Long enough to read the armed label and double-click,
/// short enough that a stale arm never surprises.
#[cfg(not(test))]
const CONFIRM_WINDOW: Duration = Duration::from_secs(5);
#[cfg(test)]
const CONFIRM_WINDOW: Duration = Duration::from_millis(300);

/// One menu row, in popup order. The config's `rows` names a subset by
/// [`ROW_NAMES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Lock,
    Logout,
    Suspend,
    Reboot,
    Poweroff,
}

/// The rows' names, in popup order: what `rows` lists and `invoke` names.
pub const ROW_NAMES: [&str; 5] = ["lock", "logout", "suspend", "reboot", "poweroff"];

/// The row `name` names, if one does.
pub(crate) fn row_index(name: &str) -> Option<usize> {
    ROW_NAMES.iter().position(|known| *known == name)
}

impl Row {
    const ALL: [Self; 5] = [
        Self::Lock,
        Self::Logout,
        Self::Suspend,
        Self::Reboot,
        Self::Poweroff,
    ];

    /// The action name (`invoke power <name>`) and the popup row's action.
    fn name(self) -> &'static str {
        ROW_NAMES[self.index()]
    }

    /// The label the popup row and the tooltip show.
    fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Logout => "Log out",
            Self::Suspend => "Suspend",
            Self::Reboot => "Reboot",
            Self::Poweroff => "Shut down",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The module's options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// The bar icon, shown with no text. None shows nothing (the module
    /// takes no space until the icon is set: see the docs' example).
    pub icon: Option<Icon>,
    /// One glyph each, drawn before its row's label instead of nothing. A
    /// row with none shows its label alone. (Popup rows only: without the
    /// `popup` feature nothing reads them.)
    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
    pub row_icons: [Option<Icon>; 5],
    /// Rows the config hid (`rows` names what is shown).
    pub hidden: [bool; 5],
    /// Per-row command overrides (`lock-command` and friends): empty runs
    /// the row's own path instead.
    pub commands: [Vec<String>; 5],
}

pub fn init(settings: &super::Settings) -> Init {
    Init::Available(Box::new(started(&settings.power, conn::system_bus_path())))
}

/// Tests only: the module started against the bus at `path` (a scripted
/// daemon's socket), the way the bar starts it against the system bus.
#[cfg(test)]
pub(super) fn start_on(path: PathBuf, settings: &Settings) -> Box<dyn Module> {
    Box::new(started(settings, path))
}

fn started(settings: &Settings, bus_path: PathBuf) -> Power {
    Power {
        icon: settings.icon.clone(),
        row_icons: settings.row_icons.clone(),
        hidden: settings.hidden,
        commands: settings.commands.clone(),
        bus_path,
        link: None,
        armed: None,
        arm_timer: None,
        staged: None,
    }
}

/// The module.
struct Power {
    icon: Option<Icon>,
    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
    row_icons: [Option<Icon>; 5],
    hidden: [bool; 5],
    commands: [Vec<String>; 5],
    /// Where the system bus is dialled when first needed (lazy: no
    /// connection until the popup first opens or an invoke needs logind).
    bus_path: PathBuf,
    link: Option<Link<Live>>,
    /// The armed destructive row, if one is.
    armed: Option<Row>,
    /// Fires at the end of [`CONFIRM_WINDOW`], disarming.
    arm_timer: Option<OneShot>,
    /// The command or scoot request a performed row staged, taken once by
    /// the loop through [`Module::take_action`].
    staged: Option<Action>,
}

impl Power {
    /// Connects the system bus on first need. Says whether there is a
    /// live session afterwards (a fresh dial that waits is not one).
    fn ensure_link(&mut self) -> bool {
        if self.link.is_none() {
            self.link = Some(Link::start_on(
                "power",
                "system bus",
                Addr::Path(self.bus_path.clone()),
                session::start,
            ));
        }
        self.link.as_ref().is_some_and(|link| link.live().is_some())
    }

    /// logind's answer for `row` (unknown with no session yet: shown).
    fn can(&self, row: Row) -> session::Can {
        let live = self.link.as_ref().and_then(Link::live);
        match row {
            Row::Suspend => live.map(|live| live.can_suspend).unwrap_or_default(),
            Row::Reboot => live.map(|live| live.can_reboot).unwrap_or_default(),
            Row::Poweroff => live.map(|live| live.can_poweroff).unwrap_or_default(),
            Row::Lock | Row::Logout => session::Can::Yes,
        }
    }

    /// Whether any suspend/reboot/shut-down row still needs logind: shown
    /// and without its command override. Without one the bus is never
    /// dialled (an all-recorder setup touches nothing real).
    fn needs_logind(&self) -> bool {
        [Row::Suspend, Row::Reboot, Row::Poweroff]
            .iter()
            .any(|row| !self.hidden[row.index()] && self.commands[row.index()].is_empty())
    }

    /// Why `row` cannot run now, or nothing when it can. Exact when the
    /// config hid it, and when its own path is missing, so a refusal says
    /// what to set rather than only what failed.
    fn availability(&self, row: Row) -> Result<(), &'static str> {
        if self.hidden[row.index()] {
            return Err("that row is hidden by power.rows");
        }
        match row {
            Row::Lock => {
                if self.commands[Row::Lock.index()].is_empty() {
                    return Err("no lock command configured (power.lock-command)");
                }
            }
            Row::Logout => {
                if self.commands[Row::Logout.index()].is_empty() && !scoot_present() {
                    return Err("no logout command configured and scoot is not reachable");
                }
            }
            Row::Suspend => {
                if self.commands[Row::Suspend.index()].is_empty()
                    && self.can(Row::Suspend).refused()
                {
                    return Err("logind reports suspend is unavailable (CanSuspend is no)");
                }
            }
            Row::Reboot => {
                if self.commands[Row::Reboot.index()].is_empty() && self.can(Row::Reboot).refused()
                {
                    return Err("logind reports reboot is unavailable (CanReboot is no)");
                }
            }
            Row::Poweroff => {
                if self.commands[Row::Poweroff.index()].is_empty()
                    && self.can(Row::Poweroff).refused()
                {
                    return Err("logind reports shut down is unavailable (CanPowerOff is no)");
                }
            }
        }
        Ok(())
    }

    /// Arms `row` (first click), or performs it (second click on the same
    /// armed row). Lock performs at once: it needs no confirm.
    fn do_row(&mut self, row: Row) -> Result<Update, InvokeError> {
        self.availability(row).map_err(InvokeError::Refused)?;
        if row == Row::Lock {
            self.disarm();
            let argv = self.commands[Row::Lock.index()].clone();
            self.staged = Some(Action::Exec(argv));
            return Ok(Update::Changed);
        }
        if self.armed == Some(row) {
            self.disarm();
            return self
                .perform(row)
                .map(|()| Update::Changed)
                .map_err(InvokeError::Refused);
        }
        // The first click asks logind too (an agent's invoke with no popup
        // open learns the same answers a popup open would) — but only
        // when a row still needs it.
        if self.needs_logind() {
            self.ensure_link();
            if let Some(live) = self.link.as_mut().and_then(Link::live_mut) {
                live.refresh();
            }
        }
        self.arm(row);
        Ok(Update::Changed)
    }

    /// Arms `row` for [`CONFIRM_WINDOW`].
    fn arm(&mut self, row: Row) {
        self.armed = Some(row);
        self.arm_timer = OneShot::after(CONFIRM_WINDOW);
    }

    /// Disarms whatever is armed. Says whether anything was.
    fn disarm(&mut self) -> bool {
        let was = self.armed.is_some();
        self.armed = None;
        self.arm_timer = None;
        was
    }

    /// Carries out `row` (already confirmed): stages its command or scoot
    /// request, or queues its logind call. Refused when logind cannot even
    /// be queued (no bus, a full flight table): the row disarmed already,
    /// so the next click re-arms rather than performing half-done.
    fn perform(&mut self, row: Row) -> Result<(), &'static str> {
        let at = row.index();
        if !self.commands[at].is_empty() {
            let argv = self.commands[at].clone();
            self.staged = Some(Action::Exec(argv));
            return Ok(());
        }
        match row {
            Row::Lock => Ok(()),
            Row::Logout => {
                self.staged = Some(Action::Scoot(crate::action::ScootAction::Quit));
                Ok(())
            }
            Row::Suspend | Row::Reboot | Row::Poweroff => {
                if !self.ensure_link() {
                    return Err("no system bus");
                }
                let queued = self
                    .link
                    .as_mut()
                    .and_then(Link::live_mut)
                    .is_some_and(|live| live.perform(row));
                if queued {
                    Ok(())
                } else {
                    Err("logind is busy; try again")
                }
            }
        }
    }

    /// The error to show: the session's last refusal, if one is.
    fn shown_error(&self) -> Option<&str> {
        self.link
            .as_ref()
            .and_then(Link::live)
            .and_then(|live| live.last_error.as_deref())
    }
}

/// Whether scoot's socket is there to quit through: `SCOOT_SOCKET`, else
/// `scoot.sock` in `XDG_RUNTIME_DIR`, present on the filesystem.
fn scoot_present() -> bool {
    crate::scoot::socket_path().is_some_and(|path| path.exists())
}

impl Module for Power {
    /// The link's fds once it exists (nothing before first use), then
    /// the arm timer while a row is armed. An idle module with nothing
    /// armed wakes nothing.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        if let Some(link) = &self.link {
            link.watch(&mut |fd, events| {
                sources.add(fd, events);
            });
        }
        if let Some(timer) = &self.arm_timer {
            sources.add(timer.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let own = self.link.as_ref().map_or(0, Link::source_count);
        if source < own {
            let changed = self.link.as_mut().is_some_and(|link| {
                link.on_ready(source, events, &mut |live, event| live.apply(event))
            });
            return if changed {
                Update::Changed
            } else {
                Update::Unchanged
            };
        }
        if source == own && self.arm_timer.is_some() {
            if let Some(timer) = self.arm_timer.take() {
                timer.drain();
            }
            return if self.disarm() {
                Update::Changed
            } else {
                Update::Unchanged
            };
        }
        Update::Unchanged
    }

    /// The icon, with no text: nothing at all without one (the module
    /// then takes no space). The tooltip names the rows shown and any
    /// failure, so a hover says what a click offers.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(icon) = &self.icon else {
            return;
        };
        view.show_icon(icon);
        let _ = write!(view.tooltip_mut(), "Power");
        let mut shown = false;
        for row in Row::ALL {
            if self.availability(row).is_err() {
                continue;
            }
            if !shown {
                let _ = write!(view.tooltip_mut(), ": ");
                shown = true;
            } else {
                let _ = write!(view.tooltip_mut(), ", ");
            }
            let _ = write!(view.tooltip_mut(), "{}", row.label());
        }
        if let Some(error) = self.shown_error() {
            if shown {
                let _ = write!(view.tooltip_mut(), "; ");
            } else {
                let _ = write!(view.tooltip_mut(), ": ");
            }
            let _ = write!(view.tooltip_mut(), "{error}");
        }
    }

    /// Carries out the row's action: the popup rows' clicks and an agent's
    /// `invoke power <row>` land here alike, so the two-step guards both.
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
            "lock" => self.do_row(Row::Lock),
            "logout" => self.do_row(Row::Logout),
            "suspend" => self.do_row(Row::Suspend),
            "reboot" => self.do_row(Row::Reboot),
            "poweroff" => self.do_row(Row::Poweroff),
            _ => Err(InvokeError::Unknown),
        }
    }

    /// The staged command or scoot request, taken once: the loop carries
    /// it out like a binding.
    fn take_action(&mut self) -> Option<Action> {
        self.staged.take()
    }

    /// The menu: one row per available action, the armed row confirming.
    /// The arm survives refills and reopens (arming lengthens its label,
    /// which resizes the popup, which reopens it: disarming on a fill
    /// would make the arm invisible); the confirm timer bounds it.
    /// Formatted without allocating (this refill sizes the popup's reused
    /// buffers; refills allocate nothing).
    #[cfg(feature = "popup")]
    fn popup(&mut self, _output: &OutputView<'_>, content: &mut crate::popup::Content) -> bool {
        if self.needs_logind() {
            self.ensure_link();
            if let Some(live) = self.link.as_mut().and_then(Link::live_mut) {
                live.refresh();
            }
        }
        if let Some(error) = self.shown_error() {
            content.text(format_args!("{error}"));
        }
        for row in Row::ALL {
            if self.availability(row).is_err() {
                continue;
            }
            let armed = self.armed == Some(row);
            // A row that performs closes the popup after its action runs;
            // a row that arms stays open for the second click.
            let closes = row == Row::Lock || armed;
            let pushed = match self.row_icons[row.index()] {
                Some(Icon::Glyph(glyph)) if armed => content.button(
                    format_args!("{glyph} {}? Click again", row.label()),
                    row.name(),
                    None,
                    true,
                    closes,
                ),
                Some(Icon::Glyph(glyph)) => content.button(
                    format_args!("{glyph} {}", row.label()),
                    row.name(),
                    None,
                    false,
                    closes,
                ),
                _ if armed => content.button(
                    format_args!("{}? Click again", row.label()),
                    row.name(),
                    None,
                    true,
                    closes,
                ),
                _ => content.button(
                    format_args!("{}", row.label()),
                    row.name(),
                    None,
                    false,
                    closes,
                ),
            };
            if !pushed {
                break;
            }
        }
        !content.is_empty()
    }

    /// What `query` reports: the rows shown, the armed one if any, and the
    /// last failure.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let rows: Vec<&str> = Row::ALL
            .iter()
            .filter(|row| self.availability(**row).is_ok())
            .map(|row| row.name())
            .collect();
        let mut value = serde_json::json!({ "rows": rows });
        if let Some(armed) = self.armed {
            value["armed"] = serde_json::Value::String(armed.name().to_owned());
        }
        if let Some(error) = self.shown_error() {
            value["error"] = serde_json::Value::String(error.to_owned());
        }
        Some(value)
    }

    /// Its view carries a tooltip: the rows and any failure.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }
}
