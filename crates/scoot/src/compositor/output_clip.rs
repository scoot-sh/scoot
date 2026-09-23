//! A window belongs to one output: it is drawn there and nowhere else, and it
//! takes input there and nowhere else.
//!
//! The core already says which output each window is on
//! ([`Placement::output`](scoot_core::Placement)), and the tree it arranges
//! from is one scrolling strip per output (see `outputs.rs`'s module doc) --
//! but a placement's *rect* is not confined to that output. A column scrolled
//! part-way off an output's edge, or a fullscreen window its column is
//! focused away from (a whole output wide, sitting in the strip beside the
//! focused column), has a rect that crosses into the neighbouring output's
//! coordinates. Drawn and hit-tested by rect alone, that window showed up on
//! the neighbour, on top of the neighbour's own windows (`apply()` maps later
//! outputs' windows after earlier ones), and took their clicks.
//!
//! So the rule, applied at every site that draws or hit-tests a window:
//!
//! - **Drawing** (`render/elements.rs`, and the ring in `decorations.rs`):
//!   an output's frame gathers only the windows placed on that output, in
//!   that output's own coordinates. The framebuffer's edge is then the clip,
//!   exactly as it always was at the outer edge of a single output.
//! - **Input** ([`State::window_element_under`], which both the pointer
//!   focus search and click-to-focus go through): a point is only ever
//!   tested against the windows placed on the output the point is on.
//!
//! **Popups go with their parent.** A menu is drawn as part of its window
//! (Smithay's `Window` element draws its popups itself), so a popup crossing
//! the shared edge is cut there -- the same thing that happens to it at a
//! single output's outer edge today, because scoot does not yet apply the
//! positioner's constraint adjustment (see
//! `docs/backlog/core/popup-constraint-adjustment.md`). Drawing the overflow
//! on the neighbour instead would put the menu over another screen's windows
//! and hand it their clicks, which is exactly the bleed this module exists
//! to stop; and input follows the pixels, so the cut part takes no input
//! either.
//!
//! # Where "the window's output" comes from, and why twice
//!
//! The render path reads [`Placement::output`](scoot_core::Placement) off the
//! arrangement it already builds each frame. The hit test runs per pointer
//! motion, where building an arrangement would be an allocation per event,
//! so it reads the output [`stamp`] recorded on the window by `apply()` --
//! from the same placement, in the same loop iteration that maps the window
//! at that placement's position, and [`unstamp`] clears it wherever the
//! window leaves the space. The stamp therefore always describes the
//! window as the `Space` currently holds it: the position the hit test
//! reads and the output it filters by were written together. The two
//! sources agree whenever the space reflects the newest arrangement, which
//! is the invariant every other reader of the space already depends on
//! (every event and action ends in `apply()`).

use std::cell::Cell;

use scoot_core::{OutputId, Rect};
use smithay::desktop::Window;
use smithay::desktop::space::SpaceElement;
use smithay::utils::{Logical, Point, Rectangle};

use super::State;

#[cfg(test)]
mod tests;

/// The output a window was last mapped onto, kept in the window's own user
/// data so it lives and dies with the window and costs no lookup table.
///
/// Non-thread-safe user data on purpose: the compositor touches windows from
/// its one event-loop thread only, and a read from any other thread gets
/// `None`, which [`State::window_element_under`] treats as "on no output" --
/// never hit, rather than hit on the wrong screen.
struct PlacedOn(Cell<Option<OutputId>>);

/// Records that `window` is now mapped on `output`. Called by `apply()` right
/// beside the `map_element` that positions it, and nowhere else; [`unstamp`]
/// is its pair at every `unmap_elem`.
///
/// Allocates once per window (the first stamp inserts the user-data slot);
/// every later stamp is a `Cell` store.
pub(super) fn stamp(window: &Window, output: OutputId) {
    window
        .user_data()
        .get_or_insert(|| PlacedOn(Cell::new(None)))
        .0
        .set(Some(output));
}

/// Forgets `window`'s output, beside every `unmap_elem` of it (`apply()`'s
/// invisible branch, `remove_window`), so the record only ever describes a
/// window the `Space` holds. An unmapped window is not on any output's
/// screen, and a stale id would outlive a move made while it was invisible
/// (moving it to another workspace or output) and name the wrong output to
/// any later reader. Allocation-free, and a no-op for a window never
/// stamped.
pub(super) fn unstamp(window: &Window) {
    if let Some(placed) = window.user_data().get::<PlacedOn>() {
        placed.0.set(None);
    }
}

/// The output `window` is mapped on, or `None` for a window that is not
/// mapped (invisible, closed, or never placed).
pub(super) fn placed_on(window: &Window) -> Option<OutputId> {
    window
        .user_data()
        .get::<PlacedOn>()
        .and_then(|placed| placed.0.get())
}

/// `rect`, in the core's global logical coordinates, relative to the origin
/// of `output` (that output's own global rectangle): what a frame of that
/// output builds its elements in, since its framebuffer starts at its own
/// origin. The identity for an output at the origin, which is where the
/// first output always is -- so a single-output session is unchanged.
///
/// Saturating rather than wrapping: both operands are bounded far inside
/// `i32` by the output and layout limits, but a saturated coordinate draws
/// off-screen, where a wrapped one could land on it.
pub(super) fn to_output_local(rect: Rect, output: Rect) -> Rect {
    Rect::new(
        rect.x.saturating_sub(output.x),
        rect.y.saturating_sub(output.y),
        rect.w,
        rect.h,
    )
}

/// Whether an output with logical rectangle `geometry` holds the point
/// `pos`: the one shared-edge rule for every "which output is this point
/// on" question -- this module's hit test and `layer_shell.rs`'s
/// `output_under` (layer-shell and session-lock hit tests, pointer-output
/// focus) both answer through it, so a point can never be on one output for
/// windows and another for bars.
///
/// Half-open on both axes ([`Rectangle::contains`] at the pinned rev:
/// `loc <= p < loc + size`), so a point on the seam between two
/// side-by-side outputs belongs to exactly one of them, the right-hand one.
pub(super) fn output_holds_point(
    geometry: Rectangle<i32, Logical>,
    pos: Point<f64, Logical>,
) -> bool {
    geometry.to_f64().contains(pos)
}

impl State {
    /// Whether `pos` lies on output `id`, by [`output_holds_point`]. `false`
    /// for an output this compositor does not have.
    fn output_holds(&self, id: OutputId, pos: Point<f64, Logical>) -> bool {
        self.outputs
            .get(id)
            .and_then(|output| self.space.output_geometry(output))
            .is_some_and(|geometry| output_holds_point(geometry, pos))
    }

    /// The output `pos` lies on, by the same rule. `None` over no output at
    /// all -- which the pointer clamp allows between outputs of uneven
    /// sizes, where nothing is drawn. The id-returning twin of
    /// `layer_shell.rs`'s `output_under`, which hands back the `Output`
    /// (an `Arc` clone) and its origin instead; both scan in creation order.
    fn output_at(&self, pos: Point<f64, Logical>) -> Option<OutputId> {
        self.outputs
            .iter_with_ids()
            .map(|(id, _)| id)
            .find(|&id| self.output_holds(id, pos))
    }

    /// The top-most window whose input region accepts `pos`, among the
    /// windows placed on the output `pos` is on, and the location it is
    /// rendered at -- `Space::element_under` with the output rule applied.
    /// Over no output, nothing; an unstamped window, never.
    ///
    /// The fast path *is* `element_under`: filtering can only remove
    /// candidates, so when Smithay's top-most answer is on the output under
    /// `pos` it is also the top-most of that output's windows, and when it
    /// finds nothing there is nothing to find. That is every point no other
    /// output's window overhangs, at the cost of one user-data read and one
    /// output's geometry (outputs never overlap -- `headless::add_output`
    /// puts each immediately right of the last -- so "its own output holds
    /// `pos`" and "it is on the output under `pos`" are the same test). Only
    /// a point under another output's overhang walks the stack again, from
    /// the top, skipping other outputs' windows; each step of that walk does
    /// one `element_location` scan, so it costs O(n^2) in the window count,
    /// confined to the bleed pixels.
    pub(super) fn window_element_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(&Window, Point<i32, Logical>)> {
        let (top, location) = self.space.element_under(pos)?;
        if placed_on(top).is_some_and(|id| self.output_holds(id, pos)) {
            return Some((top, location));
        }
        let output = self.output_at(pos)?;
        self.space
            .elements()
            .rev()
            .filter(|window| placed_on(window) == Some(output))
            .find_map(|window| {
                // `InnerElement::render_location` and `InnerElement::bbox`
                // at the pinned rev, rebuilt from the one lookup the public
                // API offers.
                let location = self.space.element_location(window)? - window.geometry().loc;
                // The trait's bbox (popups included), not the inherent
                // `Window::bbox` (the toplevel tree only).
                let mut bbox = SpaceElement::bbox(window);
                bbox.loc += location;
                (bbox.to_f64().contains(pos)
                    && window.is_in_input_region(&(pos - location.to_f64())))
                .then_some((window, location))
            })
    }
}
