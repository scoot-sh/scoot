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
//! - **[`Module::on_input`]** takes a pointer input in the module's own
//!   span ([`Input`]: a click or a scroll, with what it needs to hit-test
//!   it, [`ClickCtx`]) and answers the [`Action`] that input means by
//!   default, or none. It never acts: the bar carries the action out
//!   ([`crate::action::perform`]), through [`Module::invoke`], which is
//!   also what a configured binding (`on-click = "next"`) calls. The
//!   workspaces and window-title modules have defaults (a click on a
//!   workspace switches to it; a click on a title activates it); a binding
//!   in the config replaces them.
//! - **[`Module::custom_draw`]** draws the module itself ([`CustomDraw`]),
//!   for the modules that draw more than plain text: the workspaces
//!   module's pill behind the active workspace, and the window title's
//!   ellipsis for a title longer than its span.
//!
//! Modules are built once at start-up as trait objects (the only allocation
//! they cost the loop), and a redraw is a handful of virtual calls. A
//! reload builds them again, except an `exec` whose table did not change:
//! it moves over with its child ([`Module::keeps`], through [`start`]).
//!
//! ## Adding one
//!
//! One file in this directory, one line in [`REGISTRY`], one Cargo feature
//! in `Cargo.toml` (so a build can leave it out), and tests through
//! [`harness`] in `<id>/tests.rs`. A module whose `init` probes for
//! something a test machine may lack (a battery, a backlight) also gives
//! its registry line a [`Spec::stand_in`], so the contract test in
//! `tests.rs` exercises it everywhere. A module that answers actions lists
//! them on its registry line ([`Spec::actions`]), so a misspelled one in the
//! config is refused when it is read, and implements [`Module::invoke`].

use std::fmt::{self, Write};
use std::os::fd::BorrowedFd;

use rustix::event::{PollFd, PollFlags};

use serde_json::Value;

use crate::action::{Action, Bindings, ModuleAction, Trigger};
pub use class::{Class, MAX_TEXT};

use crate::density::Scale;
use crate::icon::{Art, Icon};
use crate::layout::Layout;
use crate::paint::{Canvas, Span};
use crate::text::Text;
use crate::theme::Theme;

#[cfg(feature = "battery")]
pub mod battery;
#[cfg(feature = "brightness")]
pub mod brightness;
#[cfg(feature = "button")]
pub mod button;
mod class;
#[cfg(feature = "clock")]
pub mod clock;
pub mod custom;
#[cfg(feature = "exec")]
pub mod exec;
#[cfg(test)]
pub mod harness;
#[cfg(feature = "network")]
pub mod network;
// A build with `button` and neither of the others reads only `Shown::text`
// of it; the rest is `push` and `exec`'s.
#[cfg(feature = "microphone")]
pub mod microphone;
#[cfg(any(feature = "button", feature = "push", feature = "exec"))]
#[cfg_attr(not(any(feature = "push", feature = "exec")), allow(dead_code))]
pub mod payload;
#[cfg(feature = "push")]
pub mod push;
#[cfg(any(feature = "push", feature = "exec"))]
mod shown;
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(any(feature = "volume", feature = "microphone"))]
pub mod volume;
#[cfg(feature = "window-title")]
pub mod window_title;
#[cfg(feature = "workspaces")]
pub mod workspaces;

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "exec"))]
mod keep_tests;

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
    /// `ext-workspace-v1` objects, and the window title's
    /// `wlr-foreign-toplevel` ones, land there, on an fd the module does
    /// not own, so `sources`/`on_ready` cannot see them.)
    fn on_dispatch(&mut self) -> Update {
        Update::Unchanged
    }

    /// A pointer input landed in this module's span: `input` says which and
    /// where, with what the module needs to hit-test it. Answers the action
    /// the input means when the config binds none (its own default), or
    /// `None`. Pure: it reads the module and never acts, so a hit test is a
    /// unit test. The default is no answer.
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        let _ = input;
        None
    }

    /// Carries out the module-defined action `action` on `output` (what a
    /// binding, a default from [`Module::on_input`] or an agent's `invoke`
    /// names), `steps` times for a scroll (1 otherwise). The default
    /// refuses every name: a module with actions lists them on its
    /// registry line and implements this.
    fn invoke(
        &mut self,
        output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = (output, action, steps);
        Err(InvokeError::Unknown)
    }

    /// An [`Action`] the module asks the bar to carry out, taken once: the
    /// loop performs it through [`crate::action::perform`] like a binding,
    /// after the turn's ready sources. The default is none. (The battery
    /// module's low-battery hook stages its `on-low` command here on a
    /// downward crossing, so the module itself never spawns: the spawn
    /// stays bounded and reaped by the bar's [`crate::spawn::Spawner`].)
    fn take_action(&mut self) -> Option<Action> {
        None
    }

    /// Whether the module answers pointer input with no binding in the
    /// config, so the bar needs a pointer for it (a workspaces click, a
    /// window-title click). A bar whose modules neither answer nor are
    /// bound takes no pointer at all.
    fn handles_input(&self) -> bool {
        false
    }

    /// Fills `content` with what this module's popup shows and answers
    /// whether it has one now: asked when the popup opens (the
    /// [`POPUP`](crate::action::POPUP) action) and again whenever the
    /// module's view changes while it is open, so a level moved elsewhere
    /// moves the slider. `false` (the default) means no popup, and closes an
    /// open one: a module that loses what it shows (the sound server went)
    /// takes its popup with it. Pure and cheap: it only reads the module.
    /// What an interaction does is a module action by name
    /// ([`Module::invoke`]), never a callback.
    #[cfg(feature = "popup")]
    fn popup(&self, output: &OutputView<'_>, content: &mut crate::popup::Content) -> bool {
        let _ = (output, content);
        false
    }

    /// The most logical pixels wide this module's span may be: longer
    /// content is cut to it, and the module draws the cut (the window
    /// title's ellipsis). `None` (the default) is whatever it measures.
    fn max_width(&self) -> Option<u32> {
        None
    }

    /// Whether the module may show a tooltip (a `tooltip` in its view), so
    /// that a bar of modules that never do takes no pointer for it. Static:
    /// whether it has one *now* is the view's. `false` (the default) is a
    /// module that writes none.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        false
    }

    /// Whether the bar draws the module in the `accent` token while the
    /// pointer is over it and it has a binding. A module that draws itself
    /// (workspaces) says no.
    fn tints_on_hover(&self) -> bool {
        true
    }

    /// A `scootbar msg set ID JSON` value for this module: the request's
    /// JSON value. Only a `push` module takes one, so the default refuses.
    fn on_set(&mut self, value: &Value) -> Result<Update, SetError> {
        let _ = value;
        Err(SetError::Unsupported)
    }

    /// What the module holds that is not its text, for `query` (`value`):
    /// the workspaces module's active workspace, a volume's percent. `None`
    /// (the default) for a module with nothing but its text.
    fn value(&self, output: &OutputView<'_>) -> Option<Value> {
        let _ = output;
        None
    }

    /// Draws the module itself, instead of the loop's plain text draw.
    /// `true` when it drew (the loop then draws nothing more for it this
    /// paint); `false` keeps the default. The workspaces and window-title
    /// modules opt in: the pill behind the active workspace, and the
    /// title's ellipsis for a title longer than its span.
    fn custom_draw(&self, ctx: &mut CustomDraw<'_, '_>) -> bool {
        let _ = ctx;
        false
    }

    /// Device pixels this module's span grows past its measured text, so
    /// a shape it draws fits: 0 for every module but the workspaces
    /// circle with `disc`, and the tray, whose view holds no text and
    /// whose icons size the span alone (the render gives an otherwise
    /// empty view room for its `span_extra`, and none without it). Cold
    /// path: asked when the module is measured (a changed view, a new
    /// scale or bar size), never per frame.
    fn span_extra(&self, measure: &Measure<'_>) -> u32 {
        let _ = measure;
        0
    }

    /// Whether this running module continues as the config's `custom`
    /// module after a reload: an `exec` whose id and `Settings` did not
    /// change keeps its child, pipe, timer and shown output, instead of
    /// starting over. Every other module declines (the default), so it is
    /// started fresh.
    fn keeps(&self, custom: &custom::Custom) -> bool {
        let _ = custom;
        false
    }
}

/// Why a `set` value was refused: the module takes none, or took this one
/// for what it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// This module takes no set value (every module but `push`).
    Unsupported,
    /// A `push` module refused the value: why.
    #[cfg(feature = "push")]
    Invalid(payload::Invalid),
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "takes no set value"),
            #[cfg(feature = "push")]
            Self::Invalid(why) => write!(f, "{why}"),
        }
    }
}

/// Why a module refused an action: said on stderr, or to the agent that
/// asked. (A build without a module that has actions — the workspaces,
/// window-title, volume, microphone, network and brightness ones — constructs none
/// but `Unknown`.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(any(
        feature = "workspaces",
        feature = "window-title",
        feature = "volume",
        feature = "microphone",
        feature = "network",
        feature = "brightness"
    )),
    allow(dead_code)
)]
pub enum InvokeError {
    /// The module has no such action.
    Unknown,
    /// The action takes a number and got none.
    NeedsArg,
    /// The action takes no number and got one.
    NoArg,
    /// The action is known but cannot run now (`why`).
    Refused(&'static str),
}

impl fmt::Display for InvokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => write!(f, "no such action"),
            Self::NeedsArg => write!(f, "takes a whole number"),
            Self::NoArg => write!(f, "takes no number"),
            Self::Refused(why) => write!(f, "{why}"),
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
    /// The actions the module defines, for a binding to name
    /// ([`Module::invoke`]); none for most modules, which take only
    /// `exec` and `scoot` bindings.
    pub actions: &'static [ActionSpec],
    /// Tests only: the module started as if `init`'s probe had found what
    /// it looks for (a fake device, a fixture), so the contract test in
    /// `tests.rs` drives its events and view on a machine without it.
    /// `None` when `init` is available on any machine the tests run on
    /// (the clock needs only a `timerfd`); the contract test fails a
    /// module that comes up unavailable there without one.
    #[cfg(test)]
    pub stand_in: Option<StandIn>,
}

/// One module-defined action: its name, and whether it takes a whole
/// number (`"activate 3"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    pub name: &'static str,
    pub arg: ArgKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(any(
        feature = "workspaces",
        feature = "window-title",
        feature = "volume",
        feature = "microphone",
        feature = "network",
        feature = "brightness"
    )),
    allow(dead_code)
)]
pub enum ArgKind {
    None,
    Required,
}

impl Spec {
    /// The action `name` of this module, if it defines one.
    pub fn action(&self, name: &str) -> Option<&ActionSpec> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// Tests only: how a [`Spec::stand_in`] starts its module.
#[cfg(test)]
pub type StandIn = fn(&Settings) -> Box<dyn Module>;

/// Every module this build has, one line each.
pub const REGISTRY: &[Spec] = &[
    #[cfg(feature = "battery")]
    Spec {
        id: battery::ID,
        init: battery::init,
        actions: battery::ACTIONS,
        // Unavailable on any machine without a battery (a desktop, a VM),
        // so the contract drives the fixture stand-in there instead.
        #[cfg(test)]
        stand_in: Some(battery::stand_in),
    },
    #[cfg(feature = "clock")]
    Spec {
        id: clock::ID,
        init: clock::init,
        actions: &[],
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "workspaces")]
    Spec {
        id: workspaces::ID,
        init: workspaces::init,
        actions: workspaces::ACTIONS,
        // Available on any machine the tests run on: without a compositor
        // it starts with an empty view, like a module still waiting for
        // its first event.
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "window-title")]
    Spec {
        id: window_title::ID,
        init: window_title::init,
        actions: window_title::ACTIONS,
        // Available on any machine the tests run on: without a compositor
        // it starts with an empty view, like a module still waiting for
        // its first event.
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "volume")]
    Spec {
        id: volume::ID,
        init: volume::init,
        actions: volume::ACTIONS,
        // Available on any machine the tests run on: without a sound
        // server it waits on the socket's directory with an empty view.
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "microphone")]
    Spec {
        id: microphone::ID,
        init: microphone::init,
        actions: microphone::ACTIONS,
        // As above, for the default source.
        #[cfg(test)]
        stand_in: None,
    },
    #[cfg(feature = "network")]
    Spec {
        id: network::ID,
        init: network::init,
        actions: network::ACTIONS,
        // Available on any machine the tests run on: without interfaces
        // past `lo` it says why, and the stand-in starts the same module
        // unrefused so the contract still drives it.
        #[cfg(test)]
        stand_in: Some(network::stand_in),
    },
    #[cfg(feature = "brightness")]
    Spec {
        id: brightness::ID,
        init: brightness::init,
        actions: brightness::ACTIONS,
        // Unavailable on any machine without a backlight (a desktop, a
        // VM), so the stand-in starts the same module on a fixture
        // backlight and the contract still drives it there.
        #[cfg(test)]
        stand_in: Some(brightness::stand_in),
    },
    #[cfg(feature = "tray")]
    Spec {
        id: tray::ID,
        init: tray::init,
        actions: tray::ACTIONS,
        // Available on any machine the tests run on (without a bus it
        // waits on the socket's directory with an empty view), and the
        // stand-in starts the same module on the scripted bus, so the
        // contract drives the connected path everywhere.
        #[cfg(test)]
        stand_in: Some(tray::stand_in),
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
    /// What the config binds to this module's pointer inputs.
    pub bindings: Bindings,
    /// Bumped each time the module reports a changed view: the render
    /// compares it with what each output shows (`crate::render`).
    pub revision: u64,
}

impl Placed {
    /// Whether pointer input can do anything here: a binding, or a default
    /// of the module's own.
    pub fn interactive(&self) -> bool {
        !self.bindings.is_empty() || self.module.handles_input()
    }

    /// Whether the bar tints it under the pointer (see
    /// [`Module::tints_on_hover`]).
    pub fn hoverable(&self) -> bool {
        !self.bindings.is_empty() && self.module.tints_on_hover()
    }

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

/// Starts every module `layout` places, in its order. An `exec` whose id
/// and table did not change is kept instead: `old` is the running bar's
/// modules, and a kept one moves over with its child, pipe, timer and
/// shown output (its bindings refreshed); what is left in `old` is for
/// the caller to drop, which kills a removed `exec`'s whole group. An
/// unavailable one is said on stderr (through `warn`) and left out: it
/// takes no space and no fd.
pub fn start(
    layout: &Layout,
    settings: &Settings,
    old: &mut Vec<Placed>,
    warn: &mut dyn FnMut(&str, &str),
) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (_, id) in layout.placed() {
        // A kept `exec`: the same id for the same table, wherever the
        // lists place it now (a move between sections or outputs keeps it
        // running, by the id rather than the position).
        if let Some(custom) = settings.custom_named(id) {
            if let Some(at) = old
                .iter()
                .position(|kept| kept.id == id && kept.module.keeps(custom))
            {
                let mut kept = old.remove(at);
                kept.bindings = settings.bindings_of(id);
                placed.push(kept);
                continue;
            }
        }
        // A built-in module, or one the config defines by name; the config
        // and the flags refuse any other id, skipped all the same.
        let init = match (find(id), settings.custom_named(id)) {
            (Some(spec), _) => (spec.init)(settings),
            (None, Some(custom)) => custom.start(),
            (None, None) => continue,
        };
        match init {
            Init::Available(module) => placed.push(Placed {
                id,
                module,
                bindings: settings.bindings_of(id),
                revision: 0,
            }),
            Init::Unavailable(why) => warn(id, &why),
        }
    }
    placed
}

/// Each module's own options, from the command line (a config file's
/// sections later). A module reads only its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    #[cfg(feature = "battery")]
    pub battery: battery::Settings,
    #[cfg(feature = "clock")]
    pub clock: clock::Settings,
    #[cfg(feature = "window-title")]
    pub window_title: window_title::Settings,
    #[cfg(feature = "workspaces")]
    pub workspaces: workspaces::Settings,
    #[cfg(feature = "volume")]
    pub volume: volume::Settings,
    #[cfg(feature = "microphone")]
    pub microphone: microphone::Settings,
    #[cfg(feature = "network")]
    pub network: network::Settings,
    #[cfg(feature = "brightness")]
    pub brightness: brightness::Settings,
    #[cfg(feature = "tray")]
    pub tray: tray::Settings,
    /// The interaction keys the config sets, by module id: only modules
    /// that bind something are listed.
    pub bindings: Vec<(&'static str, Bindings)>,
    /// The modules the config defines by name (`[button.NAME]`, ...), placed
    /// by their names in the layout like the built-in ones.
    pub custom: Vec<custom::Custom>,
}

impl Settings {
    /// The module the config defines as `id`, if it does.
    pub fn custom_named(&self, id: &str) -> Option<&custom::Custom> {
        self.custom.iter().find(|custom| custom.id == id)
    }

    /// The bindings module `id` has (none for a module the config leaves
    /// alone).
    pub fn bindings_of(&self, id: &str) -> Bindings {
        self.bindings
            .iter()
            .find(|(module, _)| *module == id)
            .map(|(_, bindings)| bindings.clone())
            .unwrap_or_default()
    }
}

/// What a module may know about the output it is asked to show on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputView<'a> {
    /// `wl_output.name` (`DP-1`), if the compositor sent one.
    pub name: Option<&'a str>,
}

/// What [`Module::span_extra`] may measure with: everything
/// [`Scene::update`](crate::render::Scene::update) measured the text with.
/// Only a module that grows its span reads it (the workspaces disc).
#[allow(dead_code)]
pub struct Measure<'a> {
    /// The output the span is measured for.
    pub output: OutputView<'a>,
    /// The view as just filled: what is measured.
    pub view: &'a View,
    /// What measures it.
    pub text: &'a Text,
    /// The em in device pixels, as measured.
    pub em: f32,
    /// The scale measured at.
    pub scale: Scale,
    /// The bar's height, device pixels.
    pub height: u32,
}

/// Where a pointer input landed in a module's span, and what the module
/// needs to hit-test it. The workspaces hit test reads it whole; the
/// window title reads which output was clicked.
#[allow(dead_code)]
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
    /// The module's span width and the bar's height, device pixels, and
    /// the scale they are at: what a module needs to place a shape it
    /// draws (the workspaces pill) the way it drew it.
    pub span_width: u32,
    pub height: u32,
    pub scale: Scale,
}

/// A pointer input for [`Module::on_input`]. (The workspaces,
/// window-title, volume, microphone, brightness and tray modules read it.)
#[cfg_attr(
    not(any(
        feature = "workspaces",
        feature = "window-title",
        feature = "volume",
        feature = "microphone",
        feature = "brightness",
        feature = "tray"
    )),
    allow(dead_code)
)]
pub struct Input<'a> {
    pub trigger: Trigger,
    pub at: &'a ClickCtx<'a>,
}

/// What a module draws itself with, for [`Module::custom_draw`]: the same
/// canvas, measurer, span and metrics the loop's plain text draw would use.
/// The workspaces and window-title modules' own draws read it.
#[allow(dead_code)]
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
    /// Whether the pointer is over this module's span: the plain draw
    /// tints with the `hover` token then, and a module that draws itself
    /// honors the same tint its own way (so truncating never changes the
    /// look, and neither does who paints it).
    pub hovered: bool,
    /// The scale the bar is drawn at: a module's own logical lengths
    /// (the pill's radius and inset) are device pixels through it.
    pub scale: Scale,
    pub theme: &'r Theme,
}

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
    /// A path or image icon, instead of a glyph one: never both.
    art: Option<Art>,
    class: Class,
}

impl View {
    pub fn clear(&mut self) {
        self.text.clear();
        self.tooltip.clear();
        self.icon = None;
        self.art = None;
        self.class = Class::Normal;
    }

    /// The text, to write into with `write!`; cut at [`MAX_TEXT`] bytes.
    pub fn text_mut(&mut self) -> &mut impl Write {
        &mut self.text
    }

    /// The tooltip (none while empty), cut at [`MAX_TEXT`] bytes. Shown
    /// after a hover delay, in a popup under the module (`daemon::popup`,
    /// and only in a build with the `popup` feature).
    ///
    /// (Allowed to be unused: a build with only the clock, or only
    /// workspaces, has no module that writes one.)
    #[allow(dead_code)]
    pub fn tooltip_mut(&mut self) -> &mut impl Write {
        &mut self.tooltip
    }

    /// A glyph drawn before the text, from the same font chain. Tests set
    /// one directly; a module shows its icon through [`View::show_icon`].
    #[cfg(test)]
    pub fn set_icon(&mut self, icon: Option<char>) {
        self.icon = icon;
    }

    /// Shows `icon` before the text, whichever kind it is (see
    /// [`crate::icon`]): a glyph is drawn through the font chain, a path or
    /// image by the bar. Replaces any icon set before.
    pub fn show_icon(&mut self, icon: &Icon) {
        match icon {
            Icon::Glyph(c) => {
                self.icon = Some(*c);
                self.art = None;
            }
            Icon::Art(art) => {
                self.icon = None;
                self.art = Some(art.clone());
            }
        }
    }

    /// The path or image icon, if the view shows one.
    pub fn art(&self) -> Option<&Art> {
        self.art.as_ref()
    }

    #[allow(dead_code)]
    pub fn set_class(&mut self, class: Class) {
        self.class = class;
    }

    pub fn text(&self) -> &str {
        &self.text.0
    }

    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
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
        self.text.0.is_empty() && self.icon.is_none() && self.art.is_none()
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
