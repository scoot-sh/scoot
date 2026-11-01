//! A popup's entrance: the slide the look's motion tokens ask for,
//! driven by the shared timeline helper.
//!
//! Pure over the `Instant`s the caller samples: this module reads no clock,
//! owns no timer and allocates nothing. The daemon asks "where is it now?"
//! on each `wl_surface.frame` callback while something moves, and stops
//! asking (and stops requesting callbacks) once the entrance has settled,
//! so an idle bar costs nothing.
//!
//! The entrance is a vertical slide away from the bar's edge (a drop): a top
//! bar's popup starts a little above its rest (toward the bar) and drops
//! down to it; a bottom bar's mirrors upward. A spring overshoots past rest
//! briefly (the offset flips sign), the way the notification cards' slide
//! does. `motion = "none"` moves nothing: the popup opens instantly, the
//! bytes it always did, with no extra room, no layers and no callbacks.

use std::time::Instant;

use scootui::motion::Timing;

use crate::bar::Edge;

/// How far a popup slides to rest, in logical pixels: smaller than the
/// notification cards' 40 (a popup is small and anchored to the bar).
pub const SLIDE_PX: u32 = 12;
/// Room the entrance keeps around the popup, in logical pixels per side:
/// the slide plus room for a spring's overshoot past rest. Zero for
/// `motion = "none"` (no room, no cost).
pub const MOTION_ROOM: u32 = 16;

/// Room the entrance needs around the popup, in logical pixels per side:
/// [`MOTION_ROOM`] when `timing` moves, zero otherwise. The surface is the
/// content plus the shadow's pad plus this; the popup rests at the combined
/// offset. `none` keeps the surface it always had.
pub fn motion_pad(timing: &Timing) -> (u32, u32, u32, u32) {
    if !timing.active() {
        return (0, 0, 0, 0);
    }
    (MOTION_ROOM, MOTION_ROOM, MOTION_ROOM, MOTION_ROOM)
}

/// The entrance's vertical offset at `now`, in logical pixels, signed:
/// negative is up (toward a top bar), positive is down. Zero at rest and
/// for `motion = "none"`. A spring overshoots: the sign flips briefly past
/// rest before it lands on zero.
pub fn entrance_dy(timing: &Timing, start: Instant, now: Instant, edge: Edge) -> f32 {
    if !timing.active() {
        return 0.0;
    }
    if timing.settled(start, now) {
        return 0.0;
    }
    let eased = timing.eased(start, now);
    let away = 1.0 - eased;
    let dy = SLIDE_PX as f32 * away;
    match edge {
        // A top bar's popup hangs below it: starts above rest (toward the
        // bar) and drops down to it.
        Edge::Top => -dy,
        // A bottom bar's popup hangs above it: starts below rest and rises.
        Edge::Bottom => dy,
    }
}

/// Whether the entrance begun at `start` has landed at `now`: true for
/// `motion = "none"` and from the deadline on.
pub fn settled(timing: &Timing, start: Instant, now: Instant) -> bool {
    timing.settled(start, now)
}

#[cfg(test)]
mod tests;
