//! What each output is meant to show: the choice for every output, and the
//! choices made for single outputs by name.
//!
//! - `scootbg set COLOR` (every output) replaces every choice: the named
//!   ones are dropped, so each output, present or plugged in later, shows
//!   that color.
//! - `scootbg set COLOR --output NAME` is a choice for outputs with that
//!   connector name. It is kept by name, so the monitor shows it again when
//!   it is unplugged and plugged back in.
//! - `clear` is the same with nothing: back to the compositor's own
//!   background. A cleared name stays in the table as "nothing", so it
//!   shows nothing even while every other output has a color.
//!
//! A named choice is only ever recorded for an output that is present when
//! it is made (the daemon refuses unknown names), so the table holds at
//! most one entry per connector name the compositor has shown: bounded by
//! the hardware, not by requests.

use crate::color::Color;

#[cfg(test)]
mod tests;

/// What one output should show: `None` is nothing, the compositor's own
/// background.
pub type Choice = Option<Color>;

#[derive(Debug, Default)]
pub struct Choices {
    all: Choice,
    named: Vec<(String, Choice)>,
}

impl Choices {
    /// Records `choice` for every output (`output` is `None`) or for the
    /// outputs named `output`.
    pub fn set(&mut self, output: Option<&str>, choice: Choice) {
        match output {
            None => {
                self.all = choice;
                self.named.clear();
            }
            Some(name) => match self.named.iter_mut().find(|(n, _)| n == name) {
                Some((_, slot)) => *slot = choice,
                None => self.named.push((name.to_owned(), choice)),
            },
        }
    }

    /// What an output named `name` (or unnamed) should show.
    pub fn for_output(&self, name: Option<&str>) -> Choice {
        name.and_then(|name| self.named.iter().find(|(n, _)| n == name))
            .map_or(self.all, |(_, choice)| *choice)
    }

    #[cfg(test)]
    pub fn named_len(&self) -> usize {
        self.named.len()
    }
}
