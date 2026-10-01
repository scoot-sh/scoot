//! The `button` module: an icon, some text, or both, and the interaction
//! keys, so a launcher or a power button needs no Rust.
//!
//! ```toml
//! left = ["workspaces", "launcher"]
//! [button.launcher]
//! icon = "\U000f0e65"
//! on-click = { exec = ["scootlaunch"] }
//! ```
//!
//! It shows what the config says and never changes: no source, no wakeup,
//! no allocation after start. A button with neither icon nor text shows
//! nothing and takes no space.

use std::fmt::Write;

use rustix::event::PollFlags;

use super::payload::Shown;
use super::{Module, OutputView, Sources, Update, View};
use crate::icon::Icon;

#[cfg(test)]
mod tests;

/// A button's options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// What it says, cut at [`super::MAX_TEXT`] bytes and with control
    /// characters made spaces (what the config's `text` becomes).
    pub text: String,
    /// An icon drawn before the text.
    pub icon: Option<Icon>,
}

impl Settings {
    /// Options with `text` as the config gave it, sanitized.
    pub fn new(text: &str, icon: Option<Icon>) -> Self {
        Self {
            text: Shown::text(text).text,
            icon,
        }
    }
}

pub struct Button {
    settings: Settings,
}

pub fn start(settings: &Settings) -> Box<dyn Module> {
    Box::new(Button {
        settings: settings.clone(),
    })
}

impl Module for Button {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        let _ = view.text_mut().write_str(&self.settings.text);
        if let Some(icon) = &self.settings.icon {
            view.show_icon(icon);
        }
    }
}
