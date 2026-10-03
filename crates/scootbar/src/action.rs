//! What a pointer input can do: the [`Trigger`]s a module answers, the
//! [`Action`] a binding names, and how the bar carries one out.
//!
//! A module's interaction keys (`on-click`, `on-right-click`,
//! `on-middle-click`, `on-scroll-up`, `on-scroll-down`) each hold an
//! action, one of three kinds:
//!
//! - **a module action**, a name the module defines (`"toggle-mute"`,
//!   `"next"`), with an optional whole-number argument (`"activate 3"`);
//! - **`{ exec = ["cmd", "arg"] }`**: a command line, run directly, never
//!   through a shell unless the user writes `sh -c` themselves;
//! - **`{ scoot = "quit" }`**: a request to scoot's own control socket
//!   (always built, not a Cargo feature: the request is one fixed line).
//!
//! An input with no configured binding asks the module for its own default
//! ([`crate::modules::Module::on_input`]: the workspaces module's click is
//! one), so a binding always wins over a default.
//!
//! **There is one way to carry an action out**, [`perform`], and it is the
//! only one: a pointer press, a scroll and (when it lands) `scootbar msg
//! invoke` all end here, so what a user does and what an agent does cannot
//! diverge.

use std::borrow::Cow;
use std::fmt;

use crate::modules::{InvokeError, Module, OutputView, Update};

#[cfg(test)]
mod tests;

/// The module action that opens a module's popup (`on-click = "popup"`): a
/// module whose registry line lists it answers [`Module::popup`], and the
/// bar, not the module, carries the action out ([`perform`] hands it to
/// [`Effects::popup`]), because a popup is a surface the bar owns.
///
/// [`Module::popup`]: crate::modules::Module::popup
pub const POPUP: &str = "popup";

/// The most arguments an `exec` command line has.
pub const MAX_EXEC_ARGS: usize = 32;
/// The longest any one argument of an `exec` command line is, in bytes.
pub const MAX_EXEC_ARG: usize = 4096;

/// The pointer input a binding answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Click,
    RightClick,
    MiddleClick,
    ScrollUp,
    ScrollDown,
}

impl Trigger {
    pub const ALL: [Self; 5] = [
        Self::Click,
        Self::RightClick,
        Self::MiddleClick,
        Self::ScrollUp,
        Self::ScrollDown,
    ];

    /// The config key that binds it, in a module's table.
    pub fn key(self) -> &'static str {
        match self {
            Self::Click => "on-click",
            Self::RightClick => "on-right-click",
            Self::MiddleClick => "on-middle-click",
            Self::ScrollUp => "on-scroll-up",
            Self::ScrollDown => "on-scroll-down",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A module-defined action: its name, and the whole number it takes, if
/// any. The names a module accepts are its registry line's
/// ([`crate::modules::Spec::actions`]), checked when the config is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAction {
    pub name: Cow<'static, str>,
    pub arg: Option<i32>,
}

impl ModuleAction {
    /// A module's own default action: a static name, nothing allocated.
    /// (Only modules with actions call it: the workspaces, window-title,
    /// volume, microphone, network, tray and media modules.)
    #[cfg_attr(
        not(any(
            feature = "workspaces",
            feature = "window-title",
            feature = "volume",
            feature = "microphone",
            feature = "network",
            feature = "tray",
            feature = "media"
        )),
        allow(dead_code)
    )]
    pub const fn new(name: &'static str, arg: Option<i32>) -> Self {
        Self {
            name: Cow::Borrowed(name),
            arg,
        }
    }
}

/// What `{ scoot = "..." }` asks scoot for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScootAction {
    /// End the session (`scoot msg action quit`).
    Quit,
}

impl ScootAction {
    /// The value in the config file, and in messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Quit => "quit",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// One thing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Module(ModuleAction),
    /// A command line: at least one argument, at most [`MAX_EXEC_ARGS`],
    /// each at most [`MAX_EXEC_ARG`] bytes (checked where it is read).
    Exec(Vec<String>),
    Scoot(ScootAction),
}

/// The five bindings of one module: the config's, where it sets any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings([Option<Action>; 5]);

impl Bindings {
    pub fn get(&self, trigger: Trigger) -> Option<&Action> {
        self.0.get(trigger.index()).and_then(Option::as_ref)
    }

    pub fn set(&mut self, trigger: Trigger, action: Action) {
        if let Some(slot) = self.0.get_mut(trigger.index()) {
            *slot = Some(action);
        }
    }

    /// Whether some trigger is bound to the module action `name`.
    #[cfg(feature = "popup")]
    pub fn binds_module_action(&self, name: &str) -> bool {
        self.0
            .iter()
            .flatten()
            .any(|action| matches!(action, Action::Module(named) if named.name == name))
    }

    /// Whether the config binds nothing.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }
}

/// What carrying an action out needs from outside the modules: the two
/// actions that leave the process. The daemon's ([`crate::daemon`])
/// spawns a child and writes to scoot's socket; a test's counts.
pub trait Effects {
    /// Runs `argv` (never through a shell). A scroll runs it once however
    /// many notches it covers: the step count is for module actions.
    fn exec(&mut self, argv: &[String]) -> Result<(), String>;
    /// Sends `action` to scoot.
    fn scoot(&mut self, action: ScootAction) -> Result<(), String>;
    /// Opens (or, when it is open, closes) the popup of the module the
    /// action is for ([`POPUP`]). The default refuses: a build without
    /// popups, or a test that has no surface.
    fn popup(&mut self) -> Result<(), String> {
        Err("this build has no popups".to_owned())
    }
}

/// Why an action did not run: said on stderr by the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum Failed {
    Module(InvokeError),
    Effect(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module(error) => write!(f, "{error}"),
            Self::Effect(why) => write!(f, "{why}"),
        }
    }
}

/// Carries `action` out for `module` (whose revision is `revision`) on
/// `output`: a module action goes to [`Module::invoke`] (bumping the
/// revision when its view changed), the others to `effects`. `steps` is a
/// scroll's coalesced step count, `None` for a click (a module then sees
/// 1).
pub fn perform(
    module: &mut dyn Module,
    revision: &mut u64,
    output: &OutputView<'_>,
    action: &Action,
    steps: Option<u32>,
    effects: &mut dyn Effects,
) -> Result<(), Failed> {
    match action {
        Action::Module(named) if named.name == POPUP => effects.popup().map_err(Failed::Effect),
        Action::Module(named) => {
            match module
                .invoke(output, named, steps.unwrap_or(1))
                .map_err(Failed::Module)?
            {
                Update::Changed => *revision = revision.wrapping_add(1),
                Update::Unchanged => {}
            }
            Ok(())
        }
        Action::Exec(argv) => effects.exec(argv).map_err(Failed::Effect),
        Action::Scoot(action) => effects.scoot(*action).map_err(Failed::Effect),
    }
}
