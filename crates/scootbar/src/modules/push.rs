//! The `push` module: a place on the bar that anything can write to with
//! `scootbar msg set ID VALUE`, and that costs nothing until it is.
//!
//! ```toml
//! right = ["status", "clock"]
//! [push.status]
//! placeholder = "..."          # shown until the first set; optional
//! ```
//!
//! ```sh
//! scootbar msg set status '{"text": "build ok", "class": "normal"}'
//! scootbar msg set status '"3 new mails"'     # a string is the text alone
//! scootbar msg set status null                # clears it
//! ```
//!
//! The value is the [update payload](super::payload) (a JSON object, a
//! string or `null`). There is no fd, no timer and no thread: the bar's
//! control socket already waits, and a `set` that changes what is shown is
//! one redraw of this module's span. Several `set`s in one turn of the loop
//! are drawn once. `set` only changes what the module shows; it cannot run
//! anything. A value that is refused (too long, not JSON, a class that does
//! not exist) leaves what was shown as it was and is answered by name.

use rustix::event::PollFlags;
use serde_json::Value;

use super::payload::{self, Shown};
use super::{Module, OutputView, SetError, Sources, Update, View};

#[cfg(test)]
mod tests;

/// A push module's options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// Shown until the first `set` (sanitized and bounded as any text).
    pub placeholder: String,
}

pub struct Push {
    shown: Shown,
}

pub fn start(settings: &Settings) -> Box<dyn Module> {
    Box::new(Push {
        shown: Shown::text(&settings.placeholder),
    })
}

impl Module for Push {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        self.shown.write(view);
    }

    fn on_set(&mut self, value: &Value) -> Result<Update, SetError> {
        let mut next = self.shown.clone();
        payload::from_value(value, &mut next).map_err(SetError::Invalid)?;
        if next == self.shown {
            return Ok(Update::Unchanged);
        }
        self.shown = next;
        Ok(Update::Changed)
    }
}
