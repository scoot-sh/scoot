//! Which frames may hand the primary plane to a client buffer: zero-copy
//! fullscreen on the GPU scanout tier.
//!
//! `tty/scanout.rs` has two flag sets per frame -- `DIRECT_FLAGS`, which lets
//! Smithay scan a client buffer out on the primary plane whatever its format
//! (`ALLOW_PRIMARY_PLANE_SCANOUT_ANY`, and why that is safe, is on that
//! constant), and `COMPOSITE_FLAGS`, which lets it do nothing of the kind.
//! This module decides, per frame and per output, which one a frame gets:
//! [`judge`] answers a [`PrimaryDirect`], and only
//! [`PrimaryDirect::Eligible`] earns the direct set.
//!
//! # The rule
//!
//! A frame is eligible when, and only when:
//!
//! 1. **the session is unlocked.** A locked frame's list is the lock screen
//!    alone (`gather_elements`), and it composites whole: the lock is never
//!    left to a plane assignment, and no buffer from behind it can be the
//!    thing on screen. Checked first, from the same `locked` that chose the
//!    element list, so the two cannot disagree about which frame this is.
//! 2. **a fullscreen window covers the output** (`State::covered_by_fullscreen`,
//!    the same question the render stack asks to hide the `top` layer). This
//!    is what keeps `ANY` away from an arbitrary bottom window: without a
//!    covering window there is no primary bit on the frame at all.
//! 3. **no capture client is streaming the output** (`Screencopy::streaming`:
//!    a session with a frame parked, or one that asked within the last
//!    second). A direct frame is not in the swapchain slot a capture reads,
//!    so each capture of a direct output forces a composite frame first;
//!    once per screenshot that is free, once per frame of a stream it
//!    measured worse than compositing throughout (more CPU, longer frame
//!    intervals for both the client and the capture), so a streamed output
//!    composites, exactly as it did before this existed.
//! 4. **no element in the frame is translucent** (`alpha() < 1.0`, which is
//!    where `wp_alpha_modifier_v1`'s multiplier lands). A translucent bottom
//!    element could only be tried over a black clear colour, where Smithay
//!    would ask the primary plane's own `alpha` property to stand in for the
//!    blend -- a rarely exercised property on the one plane every driver
//!    treats specially. Compositing is exact; that is the one worth trusting.
//! 5. **no element in the frame is a rounded window** (`Rounded`). `Rounded`
//!    forwards `underlying_storage`, so the buffer Smithay would scan out is
//!    the *unclipped* one and the corners would be lost, not approximated. A
//!    covering fullscreen window is never rounded (`window_elements`), so
//!    this cannot refuse a frame that should go direct; it makes "a rounded
//!    window never reaches the primary" a property of this check rather than
//!    of an argument about which element ends up bottom-most.
//!
//! 4 and 5 scan the whole list rather than the one element Smithay would
//! pick, because *which* element that is is Smithay's decision (the bottom
//! visible one, with everything above it on its own plane). Scanning all of
//! them costs nothing that matters -- a handful of elements, no allocation
//! -- and almost never refuses a frame that could have gone direct: a
//! translucent or rounded element *above* the candidate is composited, which
//! already stops Smithay trying the primary. The one exception is below it:
//! a `background`/`bottom` layer surface under the covering window (a
//! wallpaper) with a `wp_alpha_modifier_v1` factor under 1.0 refuses the
//! frame, although Smithay would have skipped that surface as hidden behind
//! an opaque window. That errs toward compositing -- a missed optimisation
//! for an unusual wallpaper, never a wrong pixel -- and is left so rather
//! than re-deriving Smithay's occlusion walk here.
//!
//! # What is deliberately *not* checked here
//!
//! Each of these is Smithay's to decide, traced at the pinned rev, and each
//! falls back to compositing the frame rather than failing it:
//!
//! - **Something composited above the window** -- an `overlay` layer surface
//!   (a notification), a popup, a cursor with no plane to ride: the primary
//!   is only tried for the last visible element, and only while every
//!   element above it was assigned a plane
//!   (`remaining_elements == 1 && primary_plane_elements.is_empty()`).
//! - **A buffer with no framebuffer** -- `wl_shm`, a single-pixel buffer, an
//!   implicit-modifier dma-buf, one the scanout device cannot import: an
//!   `Err` in `element_config`, cached, the element composites.
//! - **A buffer that does not cover the output, or a viewport, crop or
//!   transform** the plane cannot express: the plane's own format list and
//!   the atomic `TEST_ONLY` commit refuse it (and Smithay refuses a
//!   non-`Normal` transform outright on a plane with no `rotation`
//!   property); a covering but not-yet-resized buffer is not tried at all
//!   over a non-black background (it does not span the output).
//!
//! # Captures
//!
//! A direct frame is not in the swapchain slot a capture reads, so it marks
//! the recording, and a capture forces one composite frame first
//! (`render::scanout`'s module doc, and
//! `State::ensure_scanout_capture_current`). This module does not change
//! that contract; it is what makes it fire in normal use -- for one-shot
//! captures. Rule 3 keeps capture *streams* out of it.

use std::time::Instant;

use smithay::backend::renderer::element::Element;
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::output::Output;

use super::elements::Elements;
use crate::compositor::State;

#[cfg(test)]
mod tests;

/// What [`judge`] decided about one frame. Every variant but
/// [`Eligible`](Self::Eligible) is a reason the frame composites whole.
///
/// A reason rather than a `bool` so the transition log line says *why* a
/// session stopped (or started) going direct, and so the tests pin which
/// rule refused, not merely that one did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrimaryDirect {
    /// The frame may hand the primary plane to a client buffer. Smithay may
    /// still composite it (see the module doc).
    Eligible,
    /// The session is locked: the lock screen composites whole.
    Locked,
    /// No fullscreen window covers the output.
    NotCovered,
    /// A capture client is streaming the output (`Screencopy::streaming`).
    Streaming,
    /// An element in the frame has an alpha below 1.0.
    Translucent,
    /// An element in the frame is a rounded window.
    Rounded,
}

impl PrimaryDirect {
    /// Whether the frame gets the direct flag set.
    pub(crate) fn allowed(self) -> bool {
        self == Self::Eligible
    }
}

/// Decides whether this frame of `output` may go primary-direct.
///
/// `locked` must be the value the element list was gathered with, and
/// `elements` that list -- `draw_frame_scanout` passes both straight
/// through. Allocation-free, and everything past the first two rules -- the
/// clock read, the capture-session scan, the element scan -- only runs on an
/// unlocked, covered output, which is the only case where it can matter.
pub(super) fn judge<R>(
    state: &State,
    output: &Output,
    locked: bool,
    elements: &[Elements<R>],
) -> PrimaryDirect
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + 'static,
{
    if locked {
        return PrimaryDirect::Locked;
    }
    if !state.covered_by_fullscreen(output) {
        return PrimaryDirect::NotCovered;
    }
    if state
        .outputs
        .id_of(output)
        .is_some_and(|id| state.screencopy.streaming(id, Instant::now()))
    {
        return PrimaryDirect::Streaming;
    }
    judge_elements(elements.iter().map(|element| {
        (
            matches!(element, Elements::RoundedSurface(_)),
            element.alpha(),
        )
    }))
}

/// Rules 4 and 5 over `(is_rounded, alpha)` per element: the part of
/// [`judge`] that reads the frame list, split out so every combination is
/// pinnable without building render elements.
///
/// A rounded element refuses before a translucent one only because the scan
/// stops at the first refusal; which of the two a frame holding both
/// reports is not a contract.
fn judge_elements(elements: impl IntoIterator<Item = (bool, f32)>) -> PrimaryDirect {
    for (rounded, alpha) in elements {
        if rounded {
            return PrimaryDirect::Rounded;
        }
        // `< 1.0`, not `!= 1.0`: Smithay multiplies the alpha-modifier
        // factor in as `multiplier / u32::MAX`, which is exactly 1.0 for the
        // unset and the fully opaque cases, and nothing produces an alpha
        // above one.
        if alpha < 1.0 {
            return PrimaryDirect::Translucent;
        }
    }
    PrimaryDirect::Eligible
}
