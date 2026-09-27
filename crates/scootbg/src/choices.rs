//! What each output is meant to show: the choice for every output, and the
//! choices made for single outputs by name.
//!
//! - `scootbg set X` (every output) replaces every choice: the named ones
//!   are dropped, so each output, present or plugged in later, shows X.
//! - `scootbg set X --output NAME` is a choice for outputs with that
//!   connector name. It is kept by name, so the monitor shows it again when
//!   it is unplugged and plugged back in.
//! - `clear` is the same with nothing: back to the compositor's own
//!   background. A cleared name stays in the table as "nothing", so it
//!   shows nothing even while every other output has a color.
//!
//! **The newest request wins, whatever order they land in.** Every choice
//! carries its request's generation (`crate::waiters`). A color lands at
//! once, but an image lands only once decoded, maybe after newer requests
//! did; an older one then changes only what nothing newer has chosen
//! ([`Choices::set`]).
//!
//! A named choice is only ever recorded for an output that is present when
//! it is requested (the daemon refuses unknown names), so the table holds
//! at most one entry per connector name the compositor has shown: bounded
//! by the hardware, not by requests.

use crate::wallpaper::Wallpaper;

#[cfg(test)]
mod tests;

/// What one output should show: `None` is nothing, the compositor's own
/// background.
pub type Choice = Option<Wallpaper>;

#[derive(Debug, Default)]
pub struct Choices {
    /// For every output, and its generation (0: none made yet).
    all: (Choice, u64),
    named: Vec<(String, Choice, u64)>,
}

impl Choices {
    /// Records `choice`, from the request of `generation`, for every
    /// output (`output` is `None`) or for the outputs named `output`, as
    /// far as nothing newer has chosen already. Returns whether it was
    /// recorded anywhere.
    pub fn set(&mut self, output: Option<&str>, choice: Choice, generation: u64) -> bool {
        if self.supersedes(output, generation) {
            return false;
        }
        match output {
            None => {
                self.all = (choice, generation);
                // Choices made for single outputs *after* this request
                // stay: they are newer.
                self.named.retain(|(_, _, made)| *made > generation);
            }
            Some(name) => match self.named.iter_mut().find(|(n, ..)| n == name) {
                Some((_, slot, made)) => {
                    *slot = choice;
                    *made = generation;
                }
                None => self.named.push((name.to_owned(), choice, generation)),
            },
        }
        true
    }

    /// Whether a request of `generation` for `output` (every output when
    /// `None`) would change nothing, because newer choices cover all it
    /// asks for: a newer every-output choice, or, for one output, a newer
    /// choice for that name.
    pub fn supersedes(&self, output: Option<&str>, generation: u64) -> bool {
        if self.all.1 > generation {
            return true;
        }
        output.is_some_and(|name| {
            self.named
                .iter()
                .any(|(n, _, made)| n == name && *made > generation)
        })
    }

    /// What an output named `name` (or unnamed) should show.
    pub fn for_output(&self, name: Option<&str>) -> Option<&Wallpaper> {
        name.and_then(|name| self.named.iter().find(|(n, ..)| n == name))
            .map_or(&self.all.0, |(_, choice, _)| choice)
            .as_ref()
    }

    #[cfg(test)]
    pub fn named_len(&self) -> usize {
        self.named.len()
    }
}
