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
//! 6. **the element Smithay would try is the covering window's own**
//!    (see [`smithay_walk`] and [`rule6`]). Smithay tries the primary only
//!    for the *last* element of its visible list, and only if that element
//!    is opaque over, and spans, the whole output or the clear colour is
//!    black or transparent (`render_frame`'s `try_assign_primary_plane`
//!    guard at the pinned rev). The visible list ends at the first element,
//!    front to back, that is opaque over and spans the output; without one
//!    it ends at the bottom-most visible element. So the frame is eligible
//!    only when that element exists, passes the guard, *and* belongs to the
//!    covering window's surface tree. Anything else -- an alpha buffer with
//!    no opaque region over a grey background (nothing passes the guard), or
//!    one over an opaque wallpaper, or over any wallpaper on a black
//!    background (the wallpaper is what Smithay would try) -- could never
//!    put the window's buffer on the primary, whatever flags the frame
//!    carried; the direct flag set adds nothing there, and the per-surface
//!    scanout feedback (`dmabuf/scanout.rs`) must not steer the client
//!    toward a scannable layout it could never use. Both were measured live
//!    on the dev VM before this rule existed: steered, never direct.
//!
//!    The walk mirrors Smithay's single-pixel-buffer substitution too: when
//!    the first opaque, output-spanning element is a single-pixel buffer (a
//!    solid-colour wallpaper, or a video player's black root under its video
//!    subsurface), Smithay drops it, makes its colour the clear colour, and
//!    the element above it becomes the last -- so an alpha window over a
//!    *black* single-pixel wallpaper is tried (and eligible), and over any
//!    other colour it is not.
//!
//!    One part of Smithay's decision is left out, and it errs toward calling
//!    a frame eligible that Smithay then composites (a steer that buys
//!    nothing; never a missed one): Smithay tries the last element only if
//!    every element above it got a plane of its own, which needs a DRM
//!    device and a `TEST_ONLY` commit to know. The case that matters is the
//!    pointer: on hardware with neither a cursor plane nor an overlay plane
//!    for it, a visible pointer over the fullscreen window is composited, so
//!    every frame it is visible composites -- while the window stays eligible
//!    and steered. It goes direct again once the pointer is hidden (a video
//!    player hides it) or moves off the output. The dev VM's virtio-gpu has a
//!    cursor plane, so this has not been seen there.
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
//!   property). A covering but not-yet-resized buffer over a non-black
//!   background is the exception: it does not span the output, so rule 6
//!   already refuses the frame, as Smithay would not try it.
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

use smithay::backend::renderer::element::{Element, Id, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer, Texture};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Physical, Rectangle, Scale};
use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};
use smithay::wayland::single_pixel_buffer::get_single_pixel_buffer;

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
    /// Nothing in the frame is opaque over, and spans, the whole output, and
    /// the clear colour is neither black nor transparent (or nothing in the
    /// frame is visible at all): Smithay would not try the primary for any
    /// element (rule 6).
    NothingOpaqueCovers,
    /// Smithay would try the primary, but for an element that is not the
    /// covering window's -- a wallpaper under a window that is not opaque,
    /// a surface above it covering the output (rule 6).
    NotTheWindow,
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
/// `elements` that list; `frame` is the size, scale and clear colour that
/// frame is rendered with -- `draw_frame_scanout` passes all of them
/// straight through. Everything past the first two rules -- the clock read,
/// the capture-session scan, the element scans -- only runs on an unlocked,
/// covered output, which is the only case where it can matter.
/// Allocation-free once `scratch` (the output's own, kept across frames)
/// has grown to the frame's region count.
pub(super) fn judge<R>(
    state: &State,
    renderer: &mut R,
    output: &Output,
    locked: bool,
    elements: &[Elements<R>],
    frame: &TriedWith,
    scratch: &mut JudgeScratch,
) -> PrimaryDirect
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + 'static,
{
    if locked {
        return PrimaryDirect::Locked;
    }
    let Some(id) = state.outputs.id_of(output) else {
        return PrimaryDirect::NotCovered;
    };
    // The covering window's root surface: `covered_by_fullscreen`'s question
    // plus the surface rule 6 needs -- an xdg toplevel's, or an X window's
    // associated one. A covering X window XWayland has not paired yet has no
    // surface tree to find Smithay's element in, and counts as not covering.
    let Some(covering) = state.fullscreen_surface(id) else {
        return PrimaryDirect::NotCovered;
    };
    if state.screencopy.streaming(id, Instant::now()) {
        return PrimaryDirect::Streaming;
    }
    let refusal = judge_elements(elements.iter().map(|element| {
        (
            matches!(element, Elements::RoundedSurface(_)),
            element.alpha(),
        )
    }));
    if refusal != PrimaryDirect::Eligible {
        return refusal;
    }
    let scale = Scale::from(frame.scale);
    let walked = smithay_walk(
        elements
            .iter()
            .map(|element| (element.geometry(scale), element.opaque_regions(scale))),
        frame.size,
        frame.clear_color,
        |index| {
            elements
                .get(index)
                .and_then(|element| solid_colour(element, renderer))
        },
        scratch,
    );
    rule6(walked.end, walked.clear_color, |index| {
        elements
            .get(index)
            .is_some_and(|element| in_tree(&covering, element.id()))
    })
}

/// What rule 6 needs to know about the frame beyond its element list: the
/// physical size and scale it is rendered at, and the clear colour
/// `DrmCompositor::render_frame` is handed -- the same three values Smithay
/// makes its own decision with.
pub(crate) struct TriedWith {
    pub(crate) size: (i32, i32),
    pub(crate) scale: f64,
    pub(crate) clear_color: Color32F,
}

/// The two rectangle lists [`smithay_walk`] works in, kept per output across
/// frames so the walk allocates only while they grow to the largest region
/// count a frame has needed -- the same reuse `DrmCompositor` makes of its
/// own `element_opaque_regions_workhouse`. A client declaring many opaque
/// rectangles grows them once, not once per frame.
#[derive(Default)]
pub(crate) struct JudgeScratch {
    /// The opaque regions of every visible element so far, in output space.
    opaque: Vec<Rectangle<i32, Physical>>,
    /// What is left of the rectangle being tested after a subtraction.
    work: Vec<Rectangle<i32, Physical>>,
}

/// Where Smithay's visible-element walk ends: the index of the last element
/// on its visible list, and whether that element is opaque over, and spans,
/// the whole output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WalkEnd {
    pub(super) index: usize,
    pub(super) spans_opaque: bool,
}

/// Smithay's visible-element walk (`render_frame`, pinned rev), over each
/// element's `(geometry, own opaque regions)` in the frame's front-to-back
/// order at physical `size`: an element outside the output, or hidden
/// behind the opaque regions of those above it, is skipped; the walk stops
/// at the first element opaque over, and spanning, the output. Answers the
/// last element on the list, or `None` when nothing is visible.
///
/// Opacity is judged exactly as Smithay judges it: the element's own opaque
/// regions are subtracted from its geometry clipped to the output *without*
/// offsetting them by the element's location, and added to the running set
/// *with* it. For a covering fullscreen window, placed at the output origin,
/// the two coincide; mirroring the arithmetic rather than correcting it keeps
/// this Smithay's answer where they do not.
fn smithay_walk<I, O>(
    elements: I,
    size: (i32, i32),
    mut clear_color: Color32F,
    mut solid: impl FnMut(usize) -> Option<Color32F>,
    scratch: &mut JudgeScratch,
) -> Walked
where
    I: IntoIterator<Item = (Rectangle<i32, Physical>, O)>,
    O: std::ops::Deref<Target = [Rectangle<i32, Physical>]>,
{
    let output = Rectangle::<i32, Physical>::from_size(size.into());
    scratch.opaque.clear();
    let mut end = None;
    for (index, (geometry, own)) in elements.into_iter().enumerate() {
        let Some(visible) = geometry.intersection(output) else {
            continue;
        };
        let mut work = std::mem::take(&mut scratch.work);
        work.clear();
        work.push(visible);
        work = Rectangle::subtract_rects_many_in_place(work, scratch.opaque.iter().copied());
        if work.is_empty() {
            scratch.work = work;
            continue;
        }
        work.clear();
        work.push(visible);
        work = Rectangle::subtract_rects_many_in_place(work, own.iter().copied());
        let opaque = work.is_empty();
        scratch.work = work;
        scratch.opaque.extend(
            own.iter()
                .map(|region| Rectangle::new(region.loc + geometry.loc, region.size))
                .filter_map(|region| region.intersection(output)),
        );
        let spans_opaque = opaque && visible.contains_rect(output);
        if spans_opaque && let Some(colour) = solid(index) {
            // Smithay's single-pixel-buffer substitution: the element is
            // dropped, its colour clears the frame, and the element above it
            // (already `end`, if any) stays the last one.
            clear_color = colour;
            break;
        }
        end = Some(WalkEnd {
            index,
            spans_opaque,
        });
        if spans_opaque {
            break;
        }
    }
    Walked { end, clear_color }
}

/// What [`smithay_walk`] ends with: the last element on the visible list,
/// and the clear colour the frame will actually be cleared to -- the one it
/// was given, or a covering single-pixel buffer's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Walked {
    pub(super) end: Option<WalkEnd>,
    pub(super) clear_color: Color32F,
}

/// The colour of `element` if its buffer is a single-pixel buffer, as
/// Smithay reads it for the substitution (`underlying_storage` ->
/// `get_single_pixel_buffer` -> `rgba32f`). Asked only for the first
/// opaque, output-spanning element, once per frame; a pointer read and a
/// user-data lookup, no allocation.
fn solid_colour<R>(element: &Elements<R>, renderer: &mut R) -> Option<Color32F>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + 'static,
{
    match element.underlying_storage(renderer)? {
        UnderlyingStorage::Wayland(buffer) => get_single_pixel_buffer(buffer)
            .ok()
            .map(|pixel| Color32F::from(pixel.rgba32f())),
        _ => None,
    }
}

/// Rule 6 over where the walk ended: Smithay's guard (the last element
/// opaque over and spanning the output, or a black or transparent clear
/// colour), then whether that element is the covering window's
/// (`in_window`, asked only when the guard passes).
fn rule6(
    end: Option<WalkEnd>,
    clear_color: Color32F,
    in_window: impl FnOnce(usize) -> bool,
) -> PrimaryDirect {
    let Some(end) = end else {
        return PrimaryDirect::NothingOpaqueCovers;
    };
    let black = clear_color.r() == 0.0 && clear_color.g() == 0.0 && clear_color.b() == 0.0;
    if !end.spans_opaque && !black && clear_color.a() != 0.0 {
        return PrimaryDirect::NothingOpaqueCovers;
    }
    if in_window(end.index) {
        PrimaryDirect::Eligible
    } else {
        PrimaryDirect::NotTheWindow
    }
}

/// Whether the element `id` names is a surface in `root`'s tree. Stops at
/// the first match; a covering window's tree is its root plus a handful of
/// subsurfaces. No allocation: an `Id` from a surface is a reference-count
/// bump.
fn in_tree(root: &WlSurface, id: &Id) -> bool {
    let mut found = false;
    with_surface_tree_downward(
        root,
        (),
        |surface, _, _| {
            if found || Id::from_wayland_resource(surface) == *id {
                found = true;
                TraversalAction::Break
            } else {
                TraversalAction::DoChildren(())
            }
        },
        |_, _, _| {},
        |_, _, _| true,
    );
    found
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
