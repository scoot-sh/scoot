//! The module API: the contract every module is written against.
//!
//! A module turns some source of events (a timer, a socket, a netlink fd)
//! into a small declarative [`View`]: text, an optional icon, a state
//! [`Class`] and an optional tooltip. It never touches pixels; the bar
//! measures, lays out and draws (`crate::render`).
//!
//! ## The life of a module
//!
//! 1. **`init`** ([`Spec::init`]) probes and returns [`Init::Available`]
//!    with the module, or [`Init::Unavailable`] saying why. An unavailable
//!    module registers no fds and takes no space, so absent hardware costs
//!    nothing. `init` must not block: a module whose set-up is slow (a bus
//!    connection, a scan) starts it without waiting, returns `Available`
//!    with an empty view, and fills it in from [`Module::on_ready`] when the
//!    answer comes. An empty view takes no space, so the module joins the
//!    bar when ready and never delays the first frame.
//! 2. **[`Module::sources`]** names the fds it wants polled, and for what.
//!    A timer is an fd too (a `timerfd`). Asked on every turn of the loop,
//!    so the set may change at any time; the fds stay the module's own.
//! 3. **[`Module::on_ready`]** handles one ready source and says whether
//!    the view changed ([`Update`]). Only then is the module's area redrawn,
//!    on every output.
//! 4. **[`Module::view`]** fills a [`View`] for one output. It is asked
//!    after an update and when an output's size or scale changes, never per
//!    frame. `output` lets a per-output module (workspaces) differ; most
//!    ignore it.
//!
//! Two hooks with defaults cover what the fd pattern cannot, so no module
//! written before them changes:
//!
//! - **[`Module::on_dispatch`]** reports state that arrived on the Wayland
//!   connection's fd, which the loop owns and dispatches centrally: the
//!   workspaces module keeps `ext-workspace-v1` objects, whose events land
//!   there rather than on a module-owned fd. The loop calls it after every
//!   dispatch, before drawing.
//! - **[`Module::on_click`]** takes a pointer button press in the module's
//!   own span, with what it needs to hit-test it ([`ClickCtx`]). Only the
//!   workspaces module implements it, with its own minimal hit test over
//!   its pill rects; the general mechanism
//!   (`docs/scootbar/backlog/pointer-and-interactions.md`) generalizes this
//!   code rather than replacing the behavior.
//! - **[`Module::custom_draw`]** draws the module itself ([`CustomDraw`]),
//!   for the one module whose look is not plain text on the bar: the
//!   workspaces module's pill behind the active workspace.
//!
//! Modules are built once at start-up as trait objects (the only allocation
//! they cost the loop), and a redraw is a handful of virtual calls.
//!
//! ## Adding one
//!
//! One file in this directory, one line in [`REGISTRY`], one Cargo feature
//! in `Cargo.toml` (so a build can leave it out), and tests through
//! [`harness`] in `<id>/tests.rs`. A module whose `init` probes for
//! something a test machine may lack (a battery, a backlight) also gives
//! its registry line a [`Spec::stand_in`], so the contract test in
//! `tests.rs` exercises it everywhere. Pointer input (`on_input`) arrives with
//! `docs/scootbar/backlog/pointer-and-interactions.md`, as a method with a
//! default, so no module written before it changes.

use std::fmt::{self, Write};
use std::os::fd::BorrowedFd;

use rustix::event::{PollFd, PollFlags};

use serde_json::Value;

use crate::layout::Layout;
use crate::paint::{Canvas, Span};
use crate::text::Text;
use crate::theme::Theme;

#[cfg(feature = "clock")]
pub mod clock;
#[cfg(test)]
pub mod harness;
#[cfg(feature = "workspaces")]
pub mod workspaces;

#[cfg(test)]
mod tests;

/// A module, once available.
pub trait Module {
    /// Adds each fd this module wants polled now, with the events it wants
    /// (`PollFlags::IN`, mostly). A module's `n`th added source is `n` in
    /// [`Module::on_ready`].
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>);

    /// Source `source` is ready with `events`. Returns whether the view
    /// changed; a module that cannot tell says it did.
    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update;

    /// Fills `view` (cleared by the caller) for `output`.
    fn view(&self, output: &OutputView<'_>, view: &mut View);

    /// State that arrived on the Wayland connection's fd since the last
    /// turn: the loop calls this after every dispatch, before drawing, and
    /// bumps the revision when it says the view changed. A module with no
    /// Wayland objects keeps the default. (The workspaces module's
    /// `ext-workspace-v1` objects land there, on an fd the module does not
    /// own, so `sources`/`on_ready` cannot see them.)
    fn on_dispatch(&mut self) -> Update {
        Update::Unchanged
    }

    /// A pointer button press landed in this module's span: `ctx` says
    /// where, with what the module needs to hit-test it. Only the
    /// workspaces module answers; the loop routes the press to whichever
    /// module's span holds it, so any other module keeps the default.
    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Update {
        let _ = ctx;
        Update::Unchanged
    }

    /// A `scootbar msg set ID JSON` value for this module: the request's
    /// JSON value. No module takes one yet, so the default refuses; the
    /// modules that will (the exec ones,
    /// `docs/scootbar/backlog/exec-push-button-modules.md`) read what they
    /// accept and answer [`Update`] like any other change.
    fn on_set(&mut self, value: &Value) -> Result<Update, SetError> {
        let _ = value;
        Err(SetError::Unsupported)
    }

    /// Draws the module itself, instead of the loop's plain text draw.
    /// `true` when it drew (the loop then draws nothing more for it this
    /// paint); `false` keeps the default. Only the workspaces module opts
    /// in, for its pill behind the active workspace.
    fn custom_draw(&self, ctx: &mut CustomDraw<'_, '_>) -> bool {
        let _ = ctx;
        false
    }
}

/// Why a `set` value was refused: the module takes none. (The exec
/// modules, `docs/scootbar/backlog/exec-push-button-modules.md`, add the
/// refusal for a value they do not accept with their first use.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// This module takes no set value (every module today).
    Unsupported,
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "takes no set value"),
        }
    }
}

/// What `init` found.
pub enum Init {
    Available(Box<dyn Module>),
    /// Why not, for one line on stderr.
    Unavailable(String),
}

/// What handling a source did to the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Update {
    Unchanged,
    Changed,
}

/// A module in the [`REGISTRY`]: its id (what `--left`, `--center` and
/// `--right` name) and how to start it.
pub struct Spec {
    pub id: &'static str,
    pub init: fn(&Settings) -> Init,
    /// Tests only: the module started as if `init`'s probe had found what
    /// it looks for (a fake device, a fixture), so the contract test in
    /// `tests.rs` drives its events and view on a machine without it.
    /// `None` when `init` is available on any machine the tests run on
    /// (the clock needs only a `timerfd`); the contract test fails a
    /// module that comes up unavailable there without one.
    #[cfg(test)]
    pub stand_in: Option<StandIn>,
}

/// Tests only: how a [`Spec::stand_in`] starts its module.
#[cfg(test)]
pub type StandIn = fn(&Settings) -> Box<dyn Module>;

/// Every module this build has, one line each.
pub const REGISTRY: &[Spec] = &[
    #[cfg(feature = "clock")]
    Spec {
        id: clock::ID,
        init: clock::init,
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "workspaces")]
    Spec {
        id: workspaces::ID,
        init: workspaces::init,
        // Available on any machine the tests run on: without a compositor
        // it starts with an empty view, like a module still waiting for
        // its first event.
        #[cfg(test)]
        stand_in: None,
    },
];

/// The registry entry for `id`.
pub fn find(id: &str) -> Option<&'static Spec> {
    REGISTRY.iter().find(|spec| spec.id == id)
}

/// A started module. Where it goes is each output's own (`crate::render::Member`).
pub struct Placed {
    /// The registry id it was started from (what `--left` and `query`
    /// name it by).
    pub id: &'static str,
    pub module: Box<dyn Module>,
    /// Bumped each time the module reports a changed view: the render
    /// compares it with what each output shows (`crate::render`).
    pub revision: u64,
}

impl Placed {
    /// Hands source `source` to the module, and bumps the revision if the
    /// view changed.
    pub fn ready(&mut self, source: usize, events: PollFlags) {
        if self.module.on_ready(source, events) == Update::Changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Hands the turn's Wayland dispatch to the module, and bumps the
    /// revision if the view changed (see [`Module::on_dispatch`]).
    pub fn dispatch(&mut self) {
        if self.module.on_dispatch() == Update::Changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

/// Starts every module `layout` places, in its order. An unavailable one
/// is said on stderr (through `warn`) and left out: it takes no space and
/// no fd.
pub fn start(
    layout: &Layout,
    settings: &Settings,
    warn: &mut dyn FnMut(&str, &str),
) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (_, id) in layout.placed() {
        // The command line refuses unknown ids; skipped all the same.
        let Some(spec) = find(id) else {
            continue;
        };
        match (spec.init)(settings) {
            Init::Available(module) => placed.push(Placed {
                id: spec.id,
                module,
                revision: 0,
            }),
            Init::Unavailable(why) => warn(spec.id, &why),
        }
    }
    placed
}

/// Each module's own options, from the command line (a config file's
/// sections later). A module reads only its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    #[cfg(feature = "clock")]
    pub clock: clock::Settings,
    #[cfg(feature = "workspaces")]
    pub workspaces: workspaces::Settings,
}

/// What a module may know about the output it is asked to show on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputView<'a> {
    /// `wl_output.name` (`DP-1`), if the compositor sent one.
    pub name: Option<&'a str>,
}

/// A pointer button press in a module's span, for [`Module::on_click`]:
/// where it landed and what the module needs to hit-test it.
#[allow(dead_code)] // Only the workspaces module reads it.
pub struct ClickCtx<'a> {
    /// The output whose bar was clicked.
    pub output: OutputView<'a>,
    /// Device pixels from the module span's left edge.
    pub x: u32,
    /// The view as drawn: the hit test walks its text.
    pub view: &'a View,
    /// What the module was measured with: the same measurer, size and
    /// padding the loop drew it with, so its hit rects match its ink.
    pub text: &'a Text,
    /// The em in device pixels, as drawn.
    pub em: f32,
    /// Device pixels either side of the module's content, as drawn.
    pub padding: u32,
}

/// What a module draws itself with, for [`Module::custom_draw`]: the same
/// canvas, measurer, span and metrics the loop's plain text draw would use.
#[allow(dead_code)] // Only the workspaces module reads it.
pub struct CustomDraw<'r, 'c> {
    /// The output being drawn.
    pub output: OutputView<'r>,
    /// The view as measured: the module draws its text.
    pub view: &'r View,
    pub canvas: &'r mut Canvas<'c>,
    pub text: &'r mut Text,
    /// This module's span, in device pixels.
    pub span: Span,
    /// The em in device pixels, as measured.
    pub em: f32,
    /// The baseline, in device pixels from the canvas's top.
    pub baseline: i64,
    /// Device pixels either side of the module's content, as measured.
    pub padding: u32,
    pub theme: &'r Theme,
}

/// A state class: how the bar colors a module's view (through the theme's
/// tokens, `crate::theme`). A module never picks colors itself. The clock
/// is always `Normal`; the others are the contract's, for the modules that
/// follow it (battery, volume), and tested through the render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Class {
    #[default]
    Normal,
    Warn,
    Urgent,
    Muted,
}

impl Class {
    /// The class's name in a `query` reply: `normal`, `warn`, `urgent` or
    /// `muted`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warn => "warn",
            Self::Urgent => "urgent",
            Self::Muted => "muted",
        }
    }
}

/// The most text a view holds, in bytes; more is cut at a character
/// boundary. Bounds what an untrusted string (a window title) can cost.
pub const MAX_TEXT: usize = 256;

/// The most fds the bar's loop polls in one turn (`crate::daemon`): the
/// Wayland connection and every placed module's sources, in fixed arrays
/// on its stack.
pub const MAX_POLL: usize = 64;

/// What the modules share of [`MAX_POLL`]: all of it but the Wayland
/// connection's fd. A layout places a module at most once, so while the
/// registry's modules together add no more than this, every source of
/// every layout is polled; `tests.rs` checks that sum. (The loop needs no
/// name for it: the modules fill its array from index 1 to the end.)
#[cfg(test)]
pub const MAX_SOURCES: usize = MAX_POLL - 1;

/// A module's declarative output for one output. Reused: the bar clears
/// and refills the same one, so its strings allocate once, at most
/// [`MAX_TEXT`] bytes each.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct View {
    text: Bounded,
    tooltip: Bounded,
    icon: Option<char>,
    class: Class,
}

impl View {
    pub fn clear(&mut self) {
        self.text.clear();
        self.tooltip.clear();
        self.icon = None;
        self.class = Class::Normal;
    }

    /// The text, to write into with `write!`; cut at [`MAX_TEXT`] bytes.
    pub fn text_mut(&mut self) -> &mut impl Write {
        &mut self.text
    }

    /// The tooltip (none while empty), cut at [`MAX_TEXT`] bytes. Drawn
    /// once `docs/scootbar/backlog/tooltips.md` lands. (The rest of this
    /// block is the contract the clock does not use: icon, class, tooltip.)
    #[allow(dead_code)]
    pub fn tooltip_mut(&mut self) -> &mut impl Write {
        &mut self.tooltip
    }

    /// A glyph drawn before the text, from the same font chain.
    pub fn set_icon(&mut self, icon: Option<char>) {
        self.icon = icon;
    }

    #[allow(dead_code)]
    pub fn set_class(&mut self, class: Class) {
        self.class = class;
    }

    pub fn text(&self) -> &str {
        &self.text.0
    }

    #[allow(dead_code)]
    pub fn tooltip(&self) -> &str {
        &self.tooltip.0
    }

    pub fn icon(&self) -> Option<char> {
        self.icon
    }

    pub fn class(&self) -> Class {
        self.class
    }

    /// Whether it shows nothing: it then takes no space.
    pub fn is_empty(&self) -> bool {
        self.text.0.is_empty() && self.icon.is_none()
    }

    /// Tests only: whether a write to the text or the tooltip was cut at
    /// [`MAX_TEXT`] since the last clear. The length alone cannot tell: a
    /// cut view is never longer than the bound either.
    #[cfg(test)]
    pub fn was_cut(&self) -> bool {
        self.text.1 || self.tooltip.1
    }
}

/// A string that stops growing at [`MAX_TEXT`] bytes: a write past it is
/// cut at the last character that fits, and reports an error so a
/// formatter stops there. In tests, it also remembers being cut.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Bounded(String, #[cfg(test)] bool);

impl Bounded {
    fn clear(&mut self) {
        self.0.clear();
        #[cfg(test)]
        {
            self.1 = false;
        }
    }
}

impl Write for Bounded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = MAX_TEXT.saturating_sub(self.0.len());
        if s.len() <= room {
            self.0.push_str(s);
            return Ok(());
        }
        let mut cut = room;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.0.push_str(s.get(..cut).unwrap_or(""));
        #[cfg(test)]
        {
            self.1 = true;
        }
        Err(fmt::Error)
    }
}

/// The fds the loop polls this turn, filled by each module's
/// [`Module::sources`]. A fixed array on the loop's stack: polling
/// allocates nothing.
pub struct Sources<'s, 'fd> {
    fds: &'s mut [PollFd<'fd>],
    /// For each fd: the module that added it and its index there.
    owners: &'s mut [(usize, usize)],
    len: &'s mut usize,
    module: usize,
    next: usize,
}

impl<'s, 'fd> Sources<'s, 'fd> {
    /// For the loop: `fds` and `owners` from index `*len` on are free, and
    /// what `module` adds lands there.
    pub fn new(
        fds: &'s mut [PollFd<'fd>],
        owners: &'s mut [(usize, usize)],
        len: &'s mut usize,
        module: usize,
    ) -> Self {
        Self {
            fds,
            owners,
            len,
            module,
            next: 0,
        }
    }

    /// Polls `fd` for `events`. Past the loop's capacity ([`MAX_POLL`] less
    /// the Wayland connection's fd, for all the modules, which the
    /// registry's together stay within) the fd is not polled, and this
    /// returns `false`.
    pub fn add(&mut self, fd: BorrowedFd<'fd>, events: PollFlags) -> bool {
        let index = *self.len;
        let source = self.next;
        self.next += 1;
        let (Some(slot), Some(owner)) = (self.fds.get_mut(index), self.owners.get_mut(index))
        else {
            return false;
        };
        *slot = PollFd::from_borrowed_fd(fd, events);
        *owner = (self.module, source);
        *self.len = index + 1;
        true
    }
}
