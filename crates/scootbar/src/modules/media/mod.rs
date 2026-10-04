//! The media module: what the player is playing, and play/pause, next and
//! previous, over MPRIS.
//!
//! A client of the players on the session bus (`org.mpris.MediaPlayer2.*`),
//! through the shared D-Bus client. It shows "artist - title" of the one
//! player worth showing, a play or pause icon for its state, and sends
//! `PlayPause`, `Next` and `Previous` to it. Nothing is shown while no
//! player is playing or paused, and nothing is polled: players appearing
//! and vanishing are `NameOwnerChanged` signals, a track or state change is
//! `PropertiesChanged`, and both are filtered by the bus, so an idle bar
//! with no player is woken by nothing (`session.rs`).
//!
//! ## Which player
//!
//! Several may run. A stopped one shows nothing. Of the rest the config's
//! `player` is shown if it is among them, else the one that most recently
//! started playing, else a paused one that played most recently (ties by
//! name, so the choice is the same every run): `player::select`. The
//! controls go to the player shown.
//!
//! ## States, like the tray's
//!
//! `Waiting` owns an inotify fd on the bus socket's directory and shows
//! nothing; `Live` owns the connection. A dead connection drops back to
//! waiting with nothing shown and dials once more at once, a bus that
//! keeps dropping the bar is left alone for a while, and a machine with no
//! session bus costs one descriptor (`crate::dbus::link`).
//!
//! ## Untrusted bytes
//!
//! Anything on the bus can claim a player name and say anything; it cannot
//! crash, hang or grow the bar, and can only fill the 8 slots and the waiting
//! list (`session.rs` says exactly what it can and cannot do). Titles and artists are cleaned and cut
//! when stored (`player.rs`), and the line drawn is cut to `max-width` with
//! an ellipsis measured in pixels (the window title's cut, shared). The art
//! URL a player names is never fetched, the position and volume are never
//! read, a skip is sent at most four times a second, so a scroll fling is
//! not thirty skipped tracks, and a run of title changes is drawn ten times
//! a second at most ([`DRAW_GAP`]), so a player rewriting its title
//! constantly costs a timer, not a redraw each.

use std::fmt::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use rustix::event::PollFlags;

use super::{
    ActionSpec, ArgKind, Class, CustomDraw, Init, Input, InvokeError, MAX_TEXT, Module, OutputView,
    Sources, Update, View,
};
use crate::action::{Action, ModuleAction, Trigger};
use crate::dbus::conn;
use crate::dbus::link::{Addr, Link};
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};

mod player;
mod session;
mod timer;

use player::{Player, Status};
use session::Live;
use timer::OneShot;

#[cfg(test)]
mod daemon_tests;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "media";

/// The actions a binding or an agent may name. None takes a number: a
/// scroll's steps are one skip, not many.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "play-pause",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "next",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "previous",
        arg: ArgKind::None,
    },
];

/// The most players held: a desktop has one or two. Past this a newcomer
/// is ignored, said once.
const MAX_PLAYERS: usize = 8;
/// The most `GetNameOwner` questions in flight at once after a `ListNames`:
/// a peer owning hundreds of MPRIS names is asked about a window at a time,
/// behind which a real player is still found.
const LOOKUP_WINDOW: usize = 8;
/// The names that could not be held (no room, or a second name of a
/// connection) remembered to be held when a slot frees; the oldest is
/// forgotten past this.
const MAX_WAITING: usize = 16;
/// A call unanswered this long is forgotten when its slot is wanted (no
/// bus times a call out by default: see `Conn::expire`). Short in tests.
#[cfg(not(test))]
const FLIGHT_TTL: Duration = Duration::from_secs(30);
#[cfg(test)]
const FLIGHT_TTL: Duration = Duration::from_millis(600);
/// A player is read (`GetAll`) no oftener than this.
const MIN_REFRESH_GAP: Duration = Duration::from_millis(50);
/// A skip (next, previous) within this of the last is refused: a scroll
/// fling arrives at up to sixty actions a second, and a skipped track is
/// not undone by scrolling back.
const SKIP_GAP: Duration = Duration::from_millis(250);

/// A change of what is shown (a title, a state, another player), in a run of
/// them, is drawn at most this often: the first after a quiet spell at once,
/// the rest held for one timer, the latest shown when it fires. A player that
/// rewrites its title or flaps between playing and paused hundreds of times a
/// second costs the bar ten redraws a second, not hundreds. The module
/// appearing or emptying (a first player, the last gone, the bus lost) is
/// never held.
const DRAW_GAP: Duration = Duration::from_millis(100);

/// The default `max-width`, in logical pixels.
pub const DEFAULT_MAX_WIDTH: u32 = 320;
/// The most `max-width` takes, in logical pixels.
pub const MAX_MAX_WIDTH: u32 = 4096;

/// The play and pause icons, as SVG path data in the default `0 0 24 24`
/// viewbox (only fills, only `M`, `L`, `H`, `V` and `z`, as the volume
/// module's).
const PLAY: &str = "M8 5v14l11-7z";
const PAUSE: &str = "M6 5h4v14H6zM14 5h4v14h-4z";

/// The module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The player to prefer, by its short name (`spotify` for
    /// `org.mpris.MediaPlayer2.spotify`, and for its `.instanceN`
    /// copies). None is no preference.
    pub player: Option<String>,
    /// The most logical pixels wide the module's span may be.
    pub max_width: u32,
    /// A static icon, when the config sets the icon keys: shown for both
    /// states instead of the built-in play and pause ones below.
    pub icon: Option<Icon>,
    /// One glyph per state, when the config sets the per-state keys:
    /// playing and paused. Each wins over the static icon for its own
    /// state. (A stopped player shows nothing, so there is no
    /// `icon-stopped`: there is nothing to draw it beside.)
    pub icon_playing: Option<Icon>,
    pub icon_paused: Option<Icon>,
    /// Whether the text is drawn beside the icon. `false` draws only the
    /// icon, with the text moved into the tooltip (which already names
    /// the player, the state and the line).
    pub show_text: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            player: None,
            max_width: DEFAULT_MAX_WIDTH,
            icon: None,
            icon_playing: None,
            icon_paused: None,
            show_text: true,
        }
    }
}

pub fn init(settings: &super::Settings) -> Init {
    let addr = match conn::bus_path() {
        Ok(path) => Addr::Path(path),
        Err(()) => {
            crate::print::warn(format_args!(
                "scootbar: media: DBUS_SESSION_BUS_ADDRESS names no filesystem path \
                 (abstract and other transports are not dialled); no media"
            ));
            Addr::Unusable
        }
    };
    Init::Available(start_with(&settings.media, addr))
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
    start_with(&settings.media, Addr::Stream(ours))
}

fn start_with(settings: &Settings, addr: Addr) -> Box<dyn Module> {
    let icon = |path: &str| {
        Vector::parse(path, ViewBox::default())
            .ok()
            .map(|vector| Icon::Art(Art::Vector(std::sync::Arc::new(vector))))
    };
    Box::new(Media {
        link: Link::start("media", addr, session::start),
        preferred: settings.player.clone(),
        max_width: settings.max_width.clamp(1, MAX_MAX_WIDTH),
        // Built-in paths that parse in the module's own tests; if one ever
        // stops parsing the state shows no icon rather than nothing
        // starting.
        icons: [icon(PLAY), icon(PAUSE)],
        icon: settings.icon.clone(),
        icon_playing: settings.icon_playing.clone(),
        icon_paused: settings.icon_paused.clone(),
        show_text: settings.show_text,
        last_skip: None,
        drawn: None,
        held: None,
    })
}

/// The module.
struct Media {
    link: Link<Live>,
    preferred: Option<String>,
    max_width: u32,
    /// Playing, paused.
    icons: [Option<Icon>; 2],
    /// A static icon from the config, shown for both states instead of
    /// the built-in ones.
    icon: Option<Icon>,
    /// One glyph per state from the config, winning over the static
    /// icon for its own state.
    icon_playing: Option<Icon>,
    icon_paused: Option<Icon>,
    show_text: bool,
    /// When the last skip was sent.
    last_skip: Option<Instant>,
    /// When a change was last reported to the bar.
    drawn: Option<Instant>,
    /// Armed while a change waits out [`DRAW_GAP`]: the view the bar
    /// asks for when it fires is the latest, whatever came meanwhile.
    held: Option<OneShot>,
}

/// What the view is drawn from, compared before and after each turn so the
/// bar redraws on a real change and nothing else: the player shown, its
/// state, and the revision of what it says.
type Shown = Option<(u64, Status, u64)>;

impl Media {
    fn shown(&self) -> Shown {
        let live = self.link.live()?;
        let player = live
            .players
            .get(live.selected(self.preferred.as_deref())?)?;
        Some((player.id, player.status, player.rev))
    }

    /// Reports a change to the bar now.
    fn draw_now(&mut self) -> Update {
        self.held = None;
        self.drawn = Some(Instant::now());
        Update::Changed
    }

    /// Reports a change that is not the module appearing or emptying: at once
    /// after a quiet spell, else held for one timer ([`DRAW_GAP`] since the
    /// last), however many follow.
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
    /// is told at once when the module appears or empties (a player arrives,
    /// the last one leaves, the bus goes: `dropped`), and any other change
    /// (a title, a state, another player shown) goes through
    /// [`Media::changed`]'s gap, so a player flapping between playing and
    /// paused costs ten draws a second like one rewriting its title.
    fn after(&mut self, before: Shown, dropped: bool) -> Update {
        let now = self.shown();
        if dropped || before.is_some() != now.is_some() {
            self.draw_now()
        } else if before != now {
            self.changed()
        } else {
            Update::Unchanged
        }
    }

    /// The player to show, if any.
    fn player(&self) -> Option<&Player> {
        let live = self.link.live()?;
        live.players.get(live.selected(self.preferred.as_deref())?)
    }

    /// The icon for `status`: the config's per-state glyph when set, else
    /// its static icon, else the built-in play or pause vector. `None`
    /// for stopped (which shows nothing) and where no icon parses.
    fn icon_for(&self, status: Status) -> Option<Icon> {
        match status {
            Status::Playing => self
                .icon_playing
                .clone()
                .or_else(|| self.icon.clone())
                .or_else(|| self.icons[0].clone()),
            Status::Paused => self
                .icon_paused
                .clone()
                .or_else(|| self.icon.clone())
                .or_else(|| self.icons[1].clone()),
            Status::Stopped => None,
        }
    }
}

/// The line shown for a player: `artist - title`, or whichever there is,
/// or the player's name when it has told nothing (a player that shows up
/// playing and sends no track is still a player).
fn line(player: &Player, out: &mut impl Write) {
    match (player.artist.is_empty(), player.title.is_empty()) {
        (false, false) => {
            let _ = write!(out, "{} - {}", player.artist, player.title);
        }
        (true, false) => {
            let _ = out.write_str(&player.title);
        }
        (false, true) => {
            let _ = out.write_str(&player.artist);
        }
        (true, true) => {
            let _ = out.write_str(player.short());
        }
    }
}

impl Module for Media {
    /// The link's fds (the bus socket, or the directory watch), then the
    /// refresh timer while a player waits out its gap, then the draw timer
    /// while a change of text is held. No other timer, ever: an idle module
    /// wakes nothing.
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
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let own = self.link.source_count();
        if source >= own {
            // A timer, past the link's sources: the refresh timer first
            // while it is armed (it lives in the session), then the draw
            // timer.
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
            if timer == 0 {
                if let Some(held) = self.held.take() {
                    held.drain();
                    return self.draw_now();
                }
            }
            return Update::Unchanged;
        }
        let before = self.shown();
        let dropped = self.link.on_ready(source, events, &mut |live, event| {
            live.apply(event);
            false
        });
        self.after(before, dropped)
    }

    /// `artist - title` with the state's icon, dimmed while paused;
    /// nothing while no player is playing or paused, so the module hides.
    /// With a config icon the glyph (or picture) stands before the text,
    /// or alone with `show-text = false` (the tooltip already names the
    /// player, the state and the line).
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(player) = self.player() else {
            return;
        };
        if self.show_text {
            line(player, view.text_mut());
        }
        let _ = write!(
            view.tooltip_mut(),
            "{} ({}): ",
            player.short(),
            player.status.name()
        );
        line(player, view.tooltip_mut());
        if let Some(icon) = self.icon_for(player.status) {
            view.show_icon(&icon);
        }
        if player.status == Status::Paused {
            view.set_class(Class::Muted);
        }
    }

    /// What `query` reports: the player shown and every player held, or
    /// nothing while nothing is shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let player = self.player()?;
        let live = self.link.live()?;
        Some(serde_json::json!({
            "player": player.short(),
            "bus_name": player.name,
            "status": player.status.name(),
            "title": player.title,
            "artist": player.artist,
            "players": live.players.iter().map(|p| serde_json::json!({
                "bus_name": p.name,
                "status": p.status.name(),
            })).collect::<Vec<_>>(),
        }))
    }

    /// A click plays or pauses, a right click or a scroll down skips to
    /// the next track, a middle click or a scroll up to the previous one;
    /// nothing while no player is shown (nothing to act on, and no warning
    /// for a click on an empty module).
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        self.player()?;
        let name = match input.trigger {
            Trigger::Click => "play-pause",
            Trigger::RightClick | Trigger::ScrollDown => "next",
            Trigger::MiddleClick | Trigger::ScrollUp => "previous",
        };
        Some(Action::Module(ModuleAction::new(name, None)))
    }

    /// Carries out `play-pause`, `next` and `previous` on the player
    /// shown: one call, never blocking the bar and never waiting for the
    /// answer (the track changes when the player says so, as a signal). A
    /// scroll's many steps are one skip, and skips are at most four a
    /// second ([`SKIP_GAP`], refused past it: said at most once a second on
    /// stderr when a scroll hits it, and to an agent's `invoke` always); a
    /// refusal says why.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = steps;
        let member = match &*action.name {
            "play-pause" => "PlayPause",
            "next" => "Next",
            "previous" => "Previous",
            _ => return Err(InvokeError::Unknown),
        };
        if action.arg.is_some() {
            return Err(InvokeError::NoArg);
        }
        if self.link.live().is_none() {
            return Err(InvokeError::Refused("no session bus"));
        }
        let Some(player) = self.player() else {
            return Err(InvokeError::Refused("no player is playing or paused"));
        };
        if !player.can_control {
            return Err(InvokeError::Refused("the player takes no commands"));
        }
        match member {
            "Next" if !player.can_go_next => {
                return Err(InvokeError::Refused("the player cannot skip forward"));
            }
            "Previous" if !player.can_go_previous => {
                return Err(InvokeError::Refused("the player cannot skip back"));
            }
            _ => {}
        }
        if member != "PlayPause" {
            let now = Instant::now();
            if self
                .last_skip
                .is_some_and(|at| now.duration_since(at) < SKIP_GAP)
            {
                return Err(InvokeError::Refused(
                    "a skip within 250 ms of the last one is ignored",
                ));
            }
            self.last_skip = Some(now);
        }
        let preferred = self.preferred.as_deref();
        if let Some(live) = self.link.live_mut() {
            if let Some(index) = live.selected(preferred) {
                live.control(index, member);
            }
        }
        Ok(Update::Unchanged)
    }

    /// Its view carries a tooltip: the uncut line and the player's name.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// A click plays or pauses with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// The span never grows past `max-width` logical pixels: a longer line
    /// is cut with an ellipsis, so the title yields the bar to the other
    /// modules.
    fn max_width(&self) -> Option<u32> {
        Some(self.max_width)
    }

    /// Draws the state's icon, then the line cut to the span with an
    /// ellipsis when it is longer; a line that fits draws the plain way
    /// (`false`). The color follows the plain draw, so cutting never
    /// changes the look.
    fn custom_draw(&self, ctx: &mut CustomDraw<'_, '_>) -> bool {
        let full = ctx.view.text();
        if full.is_empty() {
            return false;
        }
        let padding = ctx.padding;
        let icon = crate::render::art_extent(ctx.text, ctx.view, ctx.em);
        let available = ctx
            .span
            .width
            .saturating_sub(padding.saturating_mul(2))
            .saturating_sub(icon);
        let mut kept = [0u8; MAX_TEXT];
        let shown = match super::ellipsis::cut(ctx.text, full, ctx.em, available, &mut kept) {
            super::ellipsis::Cut::Fits => return false,
            super::ellipsis::Cut::Blank => "",
            super::ellipsis::Cut::Shown(shown) => shown,
        };
        let color = if ctx.hovered {
            ctx.theme.hover
        } else {
            ctx.theme.class(ctx.view.class())
        };
        let mut x = i64::from(ctx.span.x) + i64::from(padding);
        if let Some(art) = ctx.view.art() {
            ctx.text
                .draw_art(ctx.canvas, art, ctx.em, x, color, ctx.span);
            x += i64::from(icon);
        }
        ctx.text.draw(
            ctx.canvas,
            None,
            shown,
            ctx.em,
            x,
            ctx.baseline,
            color,
            ctx.span,
        );
        true
    }
}
