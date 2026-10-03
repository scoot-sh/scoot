//! When a tooltip shows and when it goes, as a pure state machine: no
//! Wayland objects and no clock of its own, so every sequence (a pointer
//! that slides across three modules, a press mid-delay, a popup the
//! compositor took back) is a unit test. The daemon turns what it returns
//! into a popup (`daemon::popup::tooltip`).
//!
//! ## One timer, and only while one is wanted
//!
//! A tooltip is due a delay after the pointer comes to rest on a module that
//! has one. [`Hover::wait`] is `Some` only in that one state; the loop turns
//! it into the `poll` timeout, so with nothing hovered, or a tooltip already
//! shown, or one dismissed, there is no timer, no wakeup and no clock read
//! (the clock is asked for only when the module under the pointer changes
//! or a deadline is pending). Motion within a module changes nothing: the
//! delay runs from entering the module, not from the last twitch.
//!
//! ## Dismissed stays dismissed until the pointer leaves the module
//!
//! A press, a scroll, a click popup opening over a tooltip that was due or
//! shown, the compositor taking the tooltip back (a session lock) or a
//! failed open all *dismiss*: the same module does not show it again until
//! the pointer has been off it, or the tooltip would reappear half a second
//! after every click. (A leave and an enter that arrive in one batch of
//! events are no absence: the machine is asked once a turn of the loop.)

use std::time::{Duration, Instant};

use crate::outputs::OutputId;

/// The module under the pointer, on an output: what a tooltip belongs to.
/// The module is the started module's index (the one `render` calls a
/// module, not a member of one bar's scene), so it is the same on every
/// output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub output: OutputId,
    pub module: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    /// Nothing hovered that has a tooltip, or none wanted.
    #[default]
    Idle,
    /// Hovering `key`; the tooltip is due at `due`.
    Waiting { key: Key, due: Instant },
    /// The tooltip for `key` is up.
    Shown(Key),
    /// `key` had its tooltip taken away; no new one until the pointer leaves.
    Dismissed(Key),
}

/// What the daemon must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Nothing,
    Show(Key),
    Hide,
}

#[derive(Debug, Default)]
pub struct Hover {
    state: State,
    /// `None`: tooltips are off.
    delay: Option<Duration>,
}

impl Hover {
    /// With `delay` before a tooltip shows; zero is off.
    pub fn new(delay: Duration) -> Self {
        Self {
            state: State::Idle,
            delay: (!delay.is_zero()).then_some(delay),
        }
    }

    /// Whether tooltips are on at all.
    pub fn enabled(&self) -> bool {
        self.delay.is_some()
    }

    /// Nothing waits, shows or is held back: the daemon skips the turn's
    /// work altogether while this holds and nothing is hovered.
    pub fn is_idle(&self) -> bool {
        self.state == State::Idle
    }

    /// Whether the tooltip is up.
    pub fn is_shown(&self) -> bool {
        matches!(self.state, State::Shown(_))
    }

    /// Forgets everything and takes a new delay (a reload).
    pub fn reset(&mut self, delay: Duration) {
        *self = Self::new(delay);
    }

    /// The module under the pointer is `hovered` (`None`: none with a
    /// tooltip), and `blocked` says a click popup is open (one popup at a
    /// time, and it wins). `clock` is read only when a deadline is set or
    /// checked.
    pub fn update(
        &mut self,
        hovered: Option<Key>,
        blocked: bool,
        clock: &mut impl FnMut() -> Instant,
    ) -> Step {
        let Some(delay) = self.delay else {
            return Step::Nothing;
        };
        let was_shown = self.is_shown();
        // What the state is about: the key it holds, if it holds one.
        let held = match self.state {
            State::Idle => None,
            State::Waiting { key, .. } | State::Shown(key) | State::Dismissed(key) => Some(key),
        };
        // Moved to another module, or off every one: start over from there.
        if held != hovered {
            self.state = State::Idle;
        }
        let Some(key) = hovered else {
            return if was_shown { Step::Hide } else { Step::Nothing };
        };
        if blocked {
            // What was about to show or showing is dismissed, not merely
            // hidden: it does not come back the moment the popup closes
            // under a pointer that has not moved. A module the pointer only
            // crossed meanwhile (nothing armed for it) is armed afresh when
            // the popup is gone.
            if matches!(self.state, State::Waiting { .. } | State::Shown(_)) {
                self.state = State::Dismissed(key);
            }
            return if was_shown { Step::Hide } else { Step::Nothing };
        }
        match self.state {
            State::Idle => {
                let Some(due) = clock().checked_add(delay) else {
                    self.state = State::Dismissed(key);
                    return Step::Nothing;
                };
                self.state = State::Waiting { key, due };
                // A tooltip still up on the module just left is owed its hide.
                if was_shown { Step::Hide } else { Step::Nothing }
            }
            State::Waiting { key, due } => {
                if clock() >= due {
                    self.state = State::Shown(key);
                    Step::Show(key)
                } else {
                    Step::Nothing
                }
            }
            State::Shown(_) | State::Dismissed(_) => Step::Nothing,
        }
    }

    /// How long until the tooltip is due: `Some` only while waiting. The
    /// loop's poll timeout (and no timeout at all otherwise).
    pub fn wait(&self, clock: &mut impl FnMut() -> Instant) -> Option<Duration> {
        match self.state {
            State::Waiting { due, .. } => Some(due.saturating_duration_since(clock())),
            _ => None,
        }
    }

    /// A press or a scroll (on the module `under`, if the pointer is over one
    /// with a tooltip), or a failed open: whatever was due or shown is held
    /// off until the pointer leaves the module, and so is a tooltip that was
    /// not yet armed for `under` (a popup was open when the pointer came, and
    /// the press closes it: the click is an interaction, and the tooltip does
    /// not follow it).
    pub fn dismiss(&mut self, under: Option<Key>) {
        match self.state {
            State::Waiting { key, .. } | State::Shown(key) => self.state = State::Dismissed(key),
            State::Idle => {
                if let Some(key) = under {
                    self.state = State::Dismissed(key);
                }
            }
            State::Dismissed(_) => {}
        }
    }

    /// The tooltip is gone without this machine having asked (the
    /// compositor's `popup_done`, a lock, the output going): the same as a
    /// dismissal of what was shown.
    pub fn gone(&mut self) {
        self.dismiss(None);
    }
}
