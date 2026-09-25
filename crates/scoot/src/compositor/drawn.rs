//! The part of its layout slot a window has actually drawn.
//!
//! The layout gives every window a slot and configures the client to that
//! size; what the client commits is its own call. Most fill the slot
//! exactly -- scoot tells every window in the layout it is tiled on all four
//! edges (see `fullscreen::set_layout_states`), which is what lets `foot`
//! and GTK stop sizing themselves as floating windows. Some still draw
//! short: a fixed-size dialog, an older client, a terminal told nothing, or
//! any client for the frame or two between a larger configure and the frame
//! that answers it. A few draw past the slot for the same frame or two while
//! a shrink is in flight.
//!
//! [`drawn_rect`] is that answer as a rect: the slot's origin (where the
//! window is mapped -- its geometry's origin lands on the slot's top-left
//! corner, so a short client stays anchored there and only its right and
//! bottom edges move) and, per axis, the smaller of the slot and the window
//! geometry it last committed. Everything that must agree with what is on
//! screen reads it:
//!
//! - the rounded clip (`render/elements.rs`'s `window_elements`), so the
//!   corners it cuts are the content's own corners;
//! - both focus-ring paths (`decorations.rs`), so the ring hugs the content
//!   rather than rounding a corner of empty slot;
//! - IPC `windows`' `rect`, so an agent clicking inside it lands on the
//!   window -- hit-testing was always by surface, so a click in the undrawn
//!   part of a slot never reached the window anyway.
//!
//! It must never feed anything that configures a client or teaches the core
//! (`apply()`'s size, `answer_fullscreen_request`, `observe_frame`): the
//! slot is the request, and this is only the answer to it.
//!
//! # Following a resize
//!
//! Read fresh on every frame from the committed state, so the clip and the
//! ring move on the frame the client's commit lands and not before: a slot
//! that grows keeps the ring hugging the old content until the client's
//! larger frame arrives (no ring around empty slot in between), and one
//! that shrinks clamps the ring to the new slot at once, exactly as before
//! this existed (the client's larger frame, still on screen for a frame or
//! two, draws over the ring rather than under it -- see the gather order in
//! `render/elements.rs`). Nothing is tracked per resize, so there is no
//! state to go stale.

use scoot_core::Rect;
use smithay::desktop::Window;

#[cfg(test)]
mod tests;

/// The part of `slot` a window whose committed geometry is `committed_w` x
/// `committed_h` logical pixels covers: `slot`'s origin, and per axis the
/// smaller of the two sizes.
///
/// A window with nothing committed yet (a zero or negative size -- a fresh
/// toplevel before its first buffer, or one that unmapped) reports the whole
/// slot, which is what it is about to be drawn into, and what every consumer
/// showed for it before this existed. No arithmetic, so no overflow: the
/// result is only ever `slot` or narrower.
pub fn clamp_to_slot(slot: Rect, committed_w: i32, committed_h: i32) -> Rect {
    if committed_w <= 0 || committed_h <= 0 {
        return slot;
    }
    Rect::new(
        slot.x,
        slot.y,
        committed_w.min(slot.w),
        committed_h.min(slot.h),
    )
}

/// [`clamp_to_slot`] for `window`, read from its committed geometry (the
/// client's `xdg_surface.set_window_geometry`, clamped by Smithay to its
/// surface tree's bounds, or those bounds when it set none). A window the
/// caller could not find reports its slot.
///
/// Two uncontended locks (the window's bounding box and its surface state),
/// no allocation: cheap enough for the per-frame paths that call it once per
/// placed window.
pub fn drawn_rect(slot: Rect, window: Option<&Window>) -> Rect {
    match window {
        Some(window) => {
            let committed = window.geometry().size;
            clamp_to_slot(slot, committed.w, committed.h)
        }
        None => slot,
    }
}
