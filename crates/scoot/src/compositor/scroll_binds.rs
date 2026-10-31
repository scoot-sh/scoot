//! Scroll binds: `"super+scroll-down" = "focus-window down"` in `[binds]`.
//!
//! Every scroll that reaches the pointer funnels through
//! [`State::emit_scroll`], which offers its frame to [`State::filter_scroll`]
//! first. A frame the table does not claim goes on to the focused client
//! unchanged; an axis a bind claims is removed from it, so the client never
//! sees a scroll the compositor acted on.
//!
//! # Rules
//!
//! - **Direction** is the sign of the axis value as clients receive it
//!   (positive vertical is `scroll-down`, positive horizontal
//!   `scroll-right`). The device's natural-scroll setting is already folded
//!   into that sign by libinput, so `scroll-down` always means "what scrolls
//!   the user's apps down"; there is no second knob.
//! - **Steps.** A source that reports detents (a wheel, `v120`) fires once per
//!   120 units, so a high-resolution wheel sending 30s accumulates to a step.
//!   A source that does not (fingers on a touchpad, `continuous`) fires once
//!   per [`DEFAULT_STEP`] units of motion (`[scroll] step`). The remainder
//!   after a step carries over, so a long swipe fires evenly.
//! - **Reset.** An accumulator starts over on a reversal of direction, on an
//!   event that matches no bind (modifier released, other direction), on a
//!   finger sequence's end, and after [`IDLE_RESET`] without an event (checked
//!   when the next event arrives; there is no timer).
//! - **Cap.** One event fires at most [`MAX_STEPS_PER_EVENT`] steps (one, for
//!   `close`, `quit` and `show-keymap`), so a huge injected delta cannot
//!   stall the compositor in a loop of actions.
//! - **Finger sequences are all or nothing.** A client that saw the start of
//!   a finger scroll gets its `axis_stop`; a client that did not never gets
//!   one. So a sequence a bind claims stays claimed, reversals and a released
//!   modifier included, until its stop; and a sequence that already reached
//!   the client is never claimed midway, even if the modifier goes down
//!   after it began. Wheels have no sequence and are decided per event.
//! - **Locked.** Binds never fire while the session is locked, like key
//!   binds: the scroll passes through (to the lock client), except that a
//!   `spawn` flagged `allow_when_locked` fires, as it does for keys.
//! - **Cost.** With no scroll bind in the table (the default) and no claimed
//!   sequence, the filter is two integer tests, plus for a finger frame a few
//!   branches keeping each axis's `passing` flag current (so a bind a reload
//!   adds mid-swipe cannot claim a swipe the client already saw). No
//!   allocation either way.

use std::time::{Duration, Instant};

use scoot_core::Action;
use smithay::backend::input::AxisSource;
use smithay::input::pointer::AxisFrame;

use super::State;
use super::bind_repeat::bind_action_repeats;
use super::keybindings::{BindFlags, Bound, Modifiers, ScrollDir};

#[cfg(test)]
mod tests;

/// Units of continuous scroll per step when `[scroll] step` is unset: four
/// wheel detents' worth (a libinput wheel click reports 15), so one deliberate
/// two-finger swipe moves a few columns rather than dozens.
pub(super) const DEFAULT_STEP: f64 = 60.0;
/// The accepted `[scroll] step` range: below the floor a touchpad's noise
/// fires binds, above the ceiling a step is out of reach.
pub(super) const MIN_STEP: f64 = 1.0;
pub(super) const MAX_STEP: f64 = 10_000.0;

/// How long an axis may sit idle before its partial step is forgotten.
const IDLE_RESET: Duration = Duration::from_millis(300);
/// How long a claimed finger sequence may go without an event before the
/// claim lapses. Fingers resting mid-swipe send nothing, so this is far
/// longer than [`IDLE_RESET`]; it only bounds a lost `axis_stop`.
const CLAIM_EXPIRY: Duration = Duration::from_secs(3);
/// The most steps one event fires; the rest of its motion is dropped.
const MAX_STEPS_PER_EVENT: u32 = 8;
/// v120 units per wheel detent.
const DETENT: f64 = 120.0;

/// What an accumulator counts: wheel detents (`v120`) or continuous units.
/// Switching between them (a different device took over the axis) starts over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Detents,
    Motion,
}

/// One axis's partial step.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accumulator {
    /// Signed motion not yet turned into a step, in `unit`s.
    amount: f64,
    unit: Unit,
    /// When the last event fed this axis.
    last: Option<Instant>,
    /// When a finger sequence on this axis was last claimed by a bind: swallow
    /// it to its stop. Expires after [`CLAIM_EXPIRY`], so a sequence whose
    /// stop never came (a touchpad unplugged mid-swipe) cannot swallow the
    /// next one.
    claimed_at: Option<Instant>,
    /// A finger sequence on this axis already reached the client: no bind may
    /// claim it now, or the client would never get its `axis_stop`.
    passing: bool,
}

impl Default for Accumulator {
    fn default() -> Self {
        Self {
            amount: 0.0,
            unit: Unit::Motion,
            last: None,
            claimed_at: None,
            passing: false,
        }
    }
}

impl Accumulator {
    /// Adds `delta` (signed, in `unit`s) and returns how many whole steps of
    /// `threshold` it completed, at most `cap`. The fraction left over is kept
    /// with its sign; steps past `cap` are dropped, not deferred.
    ///
    /// A non-finite delta feeds nothing. Starts over first when the axis sat
    /// idle past [`IDLE_RESET`], when `unit` changed, or when `delta` runs
    /// against the partial step already held (a reversal).
    pub(super) fn feed(
        &mut self,
        delta: f64,
        unit: Unit,
        threshold: f64,
        cap: u32,
        now: Instant,
    ) -> u32 {
        if !delta.is_finite() || delta == 0.0 || !threshold.is_finite() || threshold <= 0.0 {
            return 0;
        }
        let idle = self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > IDLE_RESET);
        if idle || self.unit != unit || self.amount * delta < 0.0 {
            self.amount = 0.0;
        }
        self.unit = unit;
        self.last = Some(now);
        self.amount += delta;
        let whole = (self.amount.abs() / threshold).floor();
        if whole < 1.0 {
            return 0;
        }
        // `fmod` is exact and keeps the dividend's sign, so the remainder is
        // always under one step -- where `amount - whole * threshold` would
        // carry the rounding error of a huge `whole` (a finite 1e300 delta)
        // into every later step.
        self.amount %= threshold;
        // `whole` can be astronomically large for a hostile delta; compare in
        // f64 before narrowing.
        if whole >= f64::from(cap) {
            cap
        } else {
            whole as u32
        }
    }

    /// Lapses a claim whose sequence went [`CLAIM_EXPIRY`] without an event.
    pub(super) fn expire_claim(&mut self, now: Instant) {
        if self
            .claimed_at
            .is_some_and(|at| now.saturating_duration_since(at) > CLAIM_EXPIRY)
        {
            self.claimed_at = None;
        }
    }

    /// Forgets the partial step. A claimed sequence stays claimed.
    pub(super) fn reset(&mut self) {
        self.amount = 0.0;
        self.last = None;
    }
}

/// The scroll-bind state of the seat: one accumulator per axis (horizontal,
/// vertical) and the live `[scroll] step`.
#[derive(Clone, Copy, Debug)]
pub(super) struct ScrollBinds {
    axes: [Accumulator; 2],
    /// Continuous units per step (`[scroll] step`).
    pub step: f64,
}

impl Default for ScrollBinds {
    fn default() -> Self {
        Self {
            axes: [Accumulator::default(); 2],
            step: DEFAULT_STEP,
        }
    }
}

impl ScrollBinds {
    fn any_claimed(&self) -> bool {
        self.axes.iter().any(|axis| axis.claimed_at.is_some())
    }

    /// The fast path's whole job (no scroll bind, no claimed sequence): keep
    /// `passing` true across a finger sequence the client is being sent, so a
    /// bind that arrives mid-swipe (a reload) cannot claim it midway and a
    /// bind that leaves (a reload) cannot strand the flag. A handful of
    /// branches, no allocation; anything but a finger frame returns at once.
    fn track_passing(&mut self, frame: &AxisFrame) {
        if frame.source != Some(AxisSource::Finger) {
            return;
        }
        let (h120, v120) = frame.v120.unwrap_or((0, 0));
        let moved = [
            frame.axis.0 != 0.0 || h120 != 0,
            frame.axis.1 != 0.0 || v120 != 0,
        ];
        let stopped = [frame.stop.0, frame.stop.1];
        for ((axis, moved), stopped) in self.axes.iter_mut().zip(moved).zip(stopped) {
            // Motion first, then the stop that may close it, as the full
            // path orders them.
            axis.passing = (axis.passing || moved) && !stopped;
        }
    }

    /// Forgets every partial step, as a reload of the table must: a step
    /// half-counted toward a bind that changed would fire the new one early.
    /// Sequences in flight keep their claim or pass state, so a swallowed
    /// sequence is still swallowed to its stop.
    pub(super) fn reset_partial_steps(&mut self) {
        for axis in &mut self.axes {
            axis.reset();
        }
    }
}

/// One axis of the frame, copied out so the loop below can edit the frame.
#[derive(Clone, Copy)]
struct AxisReading {
    value: f64,
    v120: i32,
    stop: bool,
}

impl AxisReading {
    fn of(frame: &AxisFrame, vertical: bool) -> Self {
        let (value, stop) = if vertical {
            (frame.axis.1, frame.stop.1)
        } else {
            (frame.axis.0, frame.stop.0)
        };
        let v120 = frame.v120.map_or(0, |v| if vertical { v.1 } else { v.0 });
        Self { value, v120, stop }
    }

    /// The direction the axis moved, if it did: wheel steps first (they are
    /// the exact signal), then the continuous value. Non-finite values move
    /// nowhere.
    fn direction(self, vertical: bool) -> Option<ScrollDir> {
        let sign = if self.v120 != 0 {
            f64::from(self.v120)
        } else if self.value.is_finite() {
            self.value
        } else {
            return None;
        };
        if sign == 0.0 {
            None
        } else if vertical {
            Some(if sign > 0.0 {
                ScrollDir::Down
            } else {
                ScrollDir::Up
            })
        } else {
            Some(if sign > 0.0 {
                ScrollDir::Right
            } else {
                ScrollDir::Left
            })
        }
    }
}

/// Removes an axis from the frame: what a claiming bind takes from the client.
fn strip(frame: &mut AxisFrame, vertical: bool) {
    if vertical {
        frame.axis.1 = 0.0;
        frame.stop.1 = false;
        if let Some(v120) = &mut frame.v120 {
            v120.1 = 0;
        }
    } else {
        frame.axis.0 = 0.0;
        frame.stop.0 = false;
        if let Some(v120) = &mut frame.v120 {
            v120.0 = 0;
        }
    }
}

/// Whether anything is left to send: a nonzero value, wheel step or stop on
/// either axis.
fn has_content(frame: &AxisFrame) -> bool {
    let (h120, v120) = frame.v120.unwrap_or((0, 0));
    frame.axis.0 != 0.0
        || frame.axis.1 != 0.0
        || h120 != 0
        || v120 != 0
        || frame.stop.0
        || frame.stop.1
}

/// Whether a scroll bind may fire: always when unlocked, and when locked
/// only a `spawn` flagged `allow_when_locked` -- the rule key binds follow
/// (see `State::act_bind`, which re-checks it at fire time).
fn bind_fires(locked: bool, action: &Action, flags: BindFlags) -> bool {
    !locked || (flags.allow_when_locked && matches!(action, Action::Spawn(_)))
}

/// An action a scroll event completed steps of, fired after the table
/// borrow ends.
struct Fire {
    action: Action,
    allow_when_locked: bool,
    steps: u32,
}

impl State {
    /// Offers one scroll frame to the scroll binds. Returns the frame to
    /// send on (with any claimed axis removed), or `None` when the binds
    /// took all of it. A frame no bind claims comes back untouched.
    pub(super) fn filter_scroll(&mut self, mut frame: AxisFrame) -> Option<AxisFrame> {
        if !self.keybindings.has_scroll_binds() && !self.scroll_binds.any_claimed() {
            self.scroll_binds.track_passing(&frame);
            return Some(frame);
        }
        let locked = self.session_lock.is_locked();
        let mods = self
            .seat
            .get_keyboard()
            .map(|keyboard| Modifiers::from(&keyboard.modifier_state()))
            .unwrap_or_default();
        let now = Instant::now();
        // Finger sequences end with an `axis_stop`, so they can be claimed
        // whole; every other source is decided per event.
        let sequence = frame.source == Some(AxisSource::Finger);
        let mut fires: [Option<Fire>; 2] = [None, None];

        for (index, vertical) in [false, true].into_iter().enumerate() {
            let reading = AxisReading::of(&frame, vertical);
            let axis = &mut self.scroll_binds.axes[index];
            axis.expire_claim(now);
            let mut consume = false;
            if let Some(dir) = reading.direction(vertical) {
                // A finger sequence the client has already seen part of is
                // never claimed midway (see the module doc).
                // Only a finger sequence has a middle to be claimed in; a
                // wheel click is decided on its own and never disturbs the
                // sequence state of the axis it shares.
                let usable = self
                    .keybindings
                    .match_scroll(dir, mods)
                    .filter(|_| !(sequence && axis.passing))
                    .and_then(|(bound, flags)| match bound {
                        Bound::Action(action) => {
                            bind_fires(locked, action, flags).then_some((action, flags))
                        }
                        Bound::ChangeVt(_) => None,
                    });
                match usable {
                    Some((action, flags)) => {
                        consume = true;
                        if sequence {
                            axis.claimed_at = Some(now);
                        }
                        let (delta, unit, threshold) = if reading.v120 != 0 {
                            (f64::from(reading.v120), Unit::Detents, DETENT)
                        } else {
                            (reading.value, Unit::Motion, self.scroll_binds.step)
                        };
                        let cap = if bind_action_repeats(action) {
                            MAX_STEPS_PER_EVENT
                        } else {
                            1
                        };
                        let steps = axis.feed(delta, unit, threshold, cap, now);
                        if steps > 0 {
                            // The clone is paid only by an event that fires.
                            fires[index] = Some(Fire {
                                action: action.clone(),
                                allow_when_locked: flags.allow_when_locked,
                                steps,
                            });
                        }
                    }
                    None if sequence && axis.claimed_at.is_some() => {
                        consume = true;
                        axis.claimed_at = Some(now);
                    }
                    None => {
                        axis.reset();
                        axis.passing |= sequence;
                    }
                }
            }
            if reading.stop {
                axis.reset();
                // A finger sequence's end: swallowed if its start was, and
                // either way the next sequence starts from nothing. Another
                // source's stop is not that sequence's.
                if sequence {
                    consume |= axis.claimed_at.is_some();
                    axis.claimed_at = None;
                    axis.passing = false;
                }
            }
            if consume {
                strip(&mut frame, vertical);
            }
        }

        for fire in fires.into_iter().flatten() {
            for _ in 1..fire.steps {
                self.act_bind(fire.action.clone(), fire.allow_when_locked);
            }
            self.act_bind(fire.action, fire.allow_when_locked);
        }
        has_content(&frame).then_some(frame)
    }
}
