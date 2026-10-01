//! What the seat's pointer is doing, as a pure state machine: no Wayland
//! objects, no outputs, no I/O, so every sequence of events (a press that
//! ends off the module, a flood of scroll ticks, a leave mid-press) is a
//! unit test. The glue (`daemon::input`) turns `wl_pointer` events into
//! calls here and carries out what they return.
//!
//! ## Clicks fire on release
//!
//! A press *arms* the module under it; the release fires the button's
//! action only if the pointer is still over that same module. A release
//! anywhere else (the pointer slid off, left the surface, or the module
//! was gone by then) does nothing, and never leaves a press armed: that
//! is how a button behaves everywhere else, and it is what makes "a
//! release after the pointer left" a non-event rather than a stray
//! action. A second button pressed while one is held disarms both, and
//! no press arms until every button is up again (a chord is not a click).
//! Only left, right and middle are buttons; the rest are ignored. The
//! surface holds an implicit grab from press to release, so the release
//! always arrives, and a `leave` (the surface went away) clears the press
//! too.
//!
//! ## Scrolling is counted in steps and released once a frame
//!
//! `wl_pointer` reports a wheel notch as `axis_value120` (120 a notch,
//! from version 8), as `axis_discrete` (whole notches, versions 5 to 7),
//! or, for a touchpad or a wheel without steps, only as a continuous
//! `axis` value in pixels. A frame (`wl_pointer.frame`) groups the events
//! of one input: when it carries a discrete count the continuous value
//! beside it is the same movement and is ignored, so one notch is one
//! step and never two. Everything is accumulated in 120ths of a step, so a
//! smooth scroll that moves a fraction at a time still adds up to steps,
//! and the remainder is kept between frames (and dropped when the
//! direction reverses, on `axis_stop`, and on leave). Down is positive.
//! Only the vertical axis scrolls: horizontal scroll is ignored.
//!
//! **One action per frame, however many events.** Steps pile up in a
//! counter (bounded at [`MAX_STEPS`], so a flood cannot grow anything),
//! and [`Pointer::take_scroll`] hands them over at most once per
//! [`SCROLL_FRAME`]: a device sending a thousand events a second costs
//! sixty actions a second at most, each carrying the steps that accrued,
//! and nothing while idle (the loop asks only while steps are waiting,
//! [`Pointer::scroll_wait`] says how long to sleep). A module action takes
//! the step count; an `exec` command runs once per action, however many
//! steps it covers.

use std::time::{Duration, Instant};

use wayland_client::protocol::wl_seat::Capability;

use crate::action::Trigger;
use crate::outputs::OutputId;

#[cfg(test)]
mod tests;

/// `BTN_LEFT`, `BTN_RIGHT` and `BTN_MIDDLE` (`linux/input-event-codes.h`).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// One step of a continuous scroll, in `axis` units (pixels): libinput's
/// 15 per wheel notch, which is also what a wheel with no `axis_value120`
/// reports.
const STEP_UNITS: f64 = 15.0;
/// A step, in the 120ths the accumulator counts in.
const STEP: i32 = 120;

/// The most steps waiting at once: what one action can carry. A flood
/// past it is dropped, not queued.
pub const MAX_STEPS: u32 = 32;

/// The shortest time between two scroll actions: one 60 Hz frame.
pub const SCROLL_FRAME: Duration = Duration::from_millis(16);

/// A mouse button the bar answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// The button an evdev code means, if the bar answers it.
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            BTN_LEFT => Some(Self::Left),
            BTN_RIGHT => Some(Self::Right),
            BTN_MIDDLE => Some(Self::Middle),
            _ => None,
        }
    }

    /// This button's bit in [`Pointer::down`].
    fn bit(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Right => 2,
            Self::Middle => 4,
        }
    }

    fn trigger(self) -> Trigger {
        match self {
            Self::Left => Trigger::Click,
            Self::Right => Trigger::RightClick,
            Self::Middle => Trigger::MiddleClick,
        }
    }
}

/// Whether a seat with these capabilities gets a pointer. Touch (and the
/// keyboard) are never taken: the bar ignores touch input entirely, and
/// never takes the keyboard, so it cannot disturb focus.
pub fn wants_pointer(capabilities: Capability) -> bool {
    capabilities.contains(Capability::Pointer)
}

/// Where the pointer is: over one output's bar, in surface-local logical
/// pixels as of its last enter or motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Focus {
    pub output: OutputId,
    pub x: f64,
    pub y: f64,
}

/// A module on an output: what a press or a release is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub output: OutputId,
    /// The member's place on that output's bar.
    pub member: usize,
}

#[derive(Debug, Clone, Copy)]
struct Armed {
    button: Button,
    target: Target,
}

#[derive(Debug, Default)]
pub struct Pointer {
    focus: Option<Focus>,
    armed: Option<Armed>,
    /// The buttons held now, a [`Button::bit`] each: a press arms only when
    /// none was, so a third button pressed after a cancelled chord (while
    /// one of the first two is still held) is not a click either.
    down: u8,
    /// The continuous `axis` movement of the frame being read, in 120ths
    /// of a step.
    continuous: i32,
    /// Its discrete movement (`axis_discrete`, `axis_value120`), in 120ths.
    discrete: i32,
    has_discrete: bool,
    /// What the frames so far left short of a step.
    residual: i32,
    /// Whole steps waiting for [`Pointer::take_scroll`]: down positive.
    pending: i32,
    /// When the last scroll action was taken.
    last: Option<Instant>,
}

impl Pointer {
    pub fn focus(&self) -> Option<Focus> {
        self.focus
    }

    /// The pointer entered `output`'s bar at `x`, `y`.
    pub fn enter(&mut self, output: OutputId, x: f64, y: f64) {
        self.clear();
        self.focus = Some(Focus { output, x, y });
    }

    pub fn motion(&mut self, x: f64, y: f64) {
        if let Some(focus) = self.focus.as_mut() {
            focus.x = x;
            focus.y = y;
        }
    }

    /// The pointer left the surface (or the surface went): nothing is over,
    /// nothing is armed, no scroll waits.
    pub fn leave(&mut self) {
        self.clear();
        self.focus = None;
    }

    /// `output`'s bar went away (removed, closed, hidden): forget it.
    pub fn forget(&mut self, output: OutputId) {
        if self.focus.is_some_and(|focus| focus.output == output) {
            self.leave();
        }
    }

    /// The modules changed under the pointer (a reload): what was armed or
    /// waiting was for modules that are not there any more, so it goes. The
    /// pointer itself has not moved, and is still over the bar, and the
    /// buttons still held still are: they stay in `down`, so a second one
    /// pressed after the reload is still a chord (no click), and each
    /// release clears its own.
    pub fn disarm(&mut self) {
        let down = self.down;
        self.clear();
        self.down = down;
    }

    fn clear(&mut self) {
        self.armed = None;
        self.down = 0;
        self.continuous = 0;
        self.discrete = 0;
        self.has_discrete = false;
        self.residual = 0;
        self.pending = 0;
    }

    /// A button went down over `target` (`None`: over no module).
    pub fn press(&mut self, code: u32, target: Option<Target>) {
        let Some(button) = Button::from_code(code) else {
            return;
        };
        let chord = self.down != 0;
        self.down |= button.bit();
        // A second button while one is held: a chord is no click.
        self.armed = if chord {
            None
        } else {
            target.map(|target| Armed { button, target })
        };
    }

    /// A button went up over `target` (`None`: over no module): the click
    /// to carry out, if the button was armed on this same module.
    pub fn release(&mut self, code: u32, target: Option<Target>) -> Option<(Trigger, Target)> {
        let button = Button::from_code(code)?;
        self.down &= !button.bit();
        let armed = self.armed.filter(|armed| armed.button == button)?;
        self.armed = None;
        (target == Some(armed.target)).then_some((button.trigger(), armed.target))
    }

    /// Continuous vertical scroll, in pixels, down positive.
    pub fn axis(&mut self, value: f64) {
        if self.focus.is_none() || !value.is_finite() {
            return;
        }
        let units = (value * f64::from(STEP) / STEP_UNITS).clamp(-1.0e6, 1.0e6) as i32;
        self.continuous = self.continuous.saturating_add(units);
    }

    /// Whole wheel notches (`axis_discrete`, versions 5 to 7).
    pub fn axis_discrete(&mut self, notches: i32) {
        self.axis_value120(notches.saturating_mul(STEP));
    }

    /// A wheel's movement in 120ths of a notch (`axis_value120`).
    pub fn axis_value120(&mut self, value120: i32) {
        if self.focus.is_none() {
            return;
        }
        self.discrete = self.discrete.saturating_add(value120);
        self.has_discrete = true;
    }

    /// The scroll ended (a finger lifted): the part of a step it left is
    /// not carried into the next one.
    pub fn axis_stop(&mut self) {
        self.residual = 0;
    }

    /// The frame of events so far is whole: turn it into steps.
    pub fn frame(&mut self) {
        let units = if self.has_discrete {
            self.discrete
        } else {
            self.continuous
        };
        self.continuous = 0;
        self.discrete = 0;
        self.has_discrete = false;
        if units == 0 {
            return;
        }
        if self.residual != 0 && (self.residual < 0) != (units < 0) {
            self.residual = 0;
        }
        let total = self.residual.saturating_add(units);
        let steps = total / STEP;
        self.residual = total - steps * STEP;
        let max = MAX_STEPS as i32;
        self.pending = self.pending.saturating_add(steps).clamp(-max, max);
    }

    /// Whether any scroll steps wait: the loop reads the clock only then.
    pub fn scroll_waiting(&self) -> bool {
        self.pending != 0
    }

    /// The scroll to carry out now, if any waits and a frame has passed
    /// since the last: the direction and how many steps it covers.
    pub fn take_scroll(&mut self, now: Instant) -> Option<(Trigger, u32)> {
        if self.pending == 0 || self.scroll_wait(now).is_some() {
            return None;
        }
        let trigger = if self.pending < 0 {
            Trigger::ScrollUp
        } else {
            Trigger::ScrollDown
        };
        let steps = self.pending.unsigned_abs();
        self.pending = 0;
        self.last = Some(now);
        Some((trigger, steps))
    }

    /// How long until a waiting scroll may be taken: `None` when none
    /// waits or it may be taken now. The loop's poll timeout while a
    /// scroll is held back (and no timeout at all otherwise).
    pub fn scroll_wait(&self, now: Instant) -> Option<Duration> {
        if self.pending == 0 {
            return None;
        }
        let due = self.last?.checked_add(SCROLL_FRAME)?;
        due.checked_duration_since(now).filter(|d| !d.is_zero())
    }
}
