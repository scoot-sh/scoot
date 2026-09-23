//! Where an `xdg_popup` goes when the place its positioner names would put
//! part of it off the screen: the positioner's `constraint_adjustment`
//! (flip, slide, resize), applied against the output the popup is drawn on.
//!
//! A window is drawn only on the output it is placed on, and its popups go
//! with it (see `output_clip.rs`), so the framebuffer's edge cuts a menu at
//! the edge *between* two outputs exactly as it does at an outer one. This
//! module is what keeps a menu that asked to be kept whole on screen from
//! being cut: the geometry the client is configured with is the positioner's,
//! adjusted into a **target rectangle** by Smithay's
//! [`PositionerState::get_unconstrained_geometry`] (flip, then slide, then
//! resize, per axis, as the protocol orders them). A popup that asked for no
//! adjustment on an axis is left exactly where its positioner put it on that
//! axis -- still cut, as the protocol requires ("the compositor will assume
//! that the child surface should not change its position on that axis").
//!
//! # The target
//!
//! The protocol leaves "constrained" to the compositor, naming the
//! compositor's "work area" as its example. Which area is the work area
//! follows from what would hide the popup:
//!
//! - **A window's popup: the output's usable area** -- the output minus every
//!   layer surface's exclusive zone, the same rectangle the layout tiles
//!   into. A window and its popups are drawn *below* the top layer (see
//!   `render/elements.rs`), so a menu slid up to the output's own edge would
//!   slide under a top bar and be hidden by it, and the bar would take its
//!   clicks.
//! - **...except for the window covering its output fullscreen**, whose
//!   target is the whole output: while it covers the output the top layer is
//!   not drawn at all (`layer_shell::above_windows`), so the area a bar
//!   reserved is the fullscreen window's own to draw a menu into. Only the
//!   window actually covering the output qualifies -- a fullscreen window its
//!   column is focused away from sits in the strip like any other, with the
//!   bars drawn.
//! - **A layer surface's popup** (a bar's dropdown, parented through
//!   `zwlr_layer_surface_v1.get_popup`): **the layer surface's whole output**.
//!   The bar itself lives in the zone it reserved, and its popups are drawn
//!   with it, at its layer -- constraining them to the usable area would push
//!   a dropdown out of its own bar.
//!
//! An empty usable area (exclusive zones that reserved the whole output)
//! falls back to the whole output: a zero-sized target would slide every
//! menu into a corner and still leave it "constrained".
//!
//! The rectangle is handed to Smithay in the coordinate space the positioner
//! works in: relative to the *immediate* parent's window geometry origin. For
//! a submenu that is the parent menu's origin, so the walk up to the root sums
//! each ancestor popup's committed position -- the same position
//! (`PopupState::geometry`, as last acked and committed) the render tree
//! places it by, so the target and the pixels cannot disagree.
//!
//! # When it is applied
//!
//! At the two points the client is told a popup's geometry: the initial
//! configure (`handlers.rs`'s `send_popup_initial_configure`, on the popup's
//! first commit) and `xdg_popup.reposition`.
//!
//! The initial configure rather than `XdgShellHandler::new_popup`: Smithay
//! fills the pending geometry from the raw positioner at `get_popup`, before
//! `new_popup` runs, and nothing reads it until the configure is sent. A
//! layer surface's popup has no parent yet at `new_popup` -- it is created
//! parentless and handed to the layer surface afterwards -- while by its
//! first commit it must have one (committing a parentless `xdg_popup` is a
//! protocol error Smithay posts in its pre-commit hook). So one site covers
//! window, layer and nested popups alike, against the parent's position as of
//! the moment the configure goes out, without a second tracking path for the
//! layer case (see `new_popup`'s doc for why that path must stay single).
//!
//! **Not re-constrained afterwards.** A `reactive` positioner asks to be
//! re-constrained when the conditions change (the parent scrolled, an output
//! was resized); scoot does not do that yet -- see
//! `docs/backlog/core/popup-reactive-reconstrain.md`.
//!
//! # When no target is known
//!
//! The geometry is left as the positioner computed it, unconstrained, which
//! is exactly what every popup got before this module existed:
//!
//! - the root window is not mapped on any output (invisible -- its popups are
//!   not drawn either);
//! - the root is neither a window nor a layer surface on an output;
//! - an ancestor popup has no parent (a popup made parentless for a layer
//!   surface that never adopted it, with a submenu of its own -- the child's
//!   commit is legal, its ancestor never committed);
//! - the ancestor chain is deeper than [`MAX_POPUP_DEPTH`];
//! - any positioner field or target coordinate is beyond [`COORDINATE_LIMIT`],
//!   which is what keeps Smithay's arithmetic inside `i32` (below).

use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{get_role, with_states};
use smithay::wayland::shell::xdg::{
    PopupCachedState, PopupSurface, PositionerState, XDG_POPUP_ROLE, XdgPopupSurfaceData,
};

use super::{State, output_clip};

#[cfg(test)]
mod tests;

/// The largest magnitude any positioner field or target coordinate may have
/// for the popup to be constrained at all: 2^24 logical pixels, far beyond
/// any real output or menu.
///
/// Smithay's positioner arithmetic is plain `i32` addition, which panics on
/// overflow in a debug build and wraps in a release one. Its own creation-time
/// `get_geometry` is already beyond scoot's reach, but the constraint pass
/// adds more terms on top of that result -- the popup's far edge
/// (`loc + size`), its distance from the target's edges, and a flipped
/// recompute -- so a positioner whose unadjusted geometry is still in range
/// could overflow here. With every input at most 2^24 in magnitude, no sum
/// in that pass exceeds a handful of such terms (under 2^28): no overflow is
/// possible, and a positioner beyond it asked for a place nothing can show
/// anyway.
pub(super) const COORDINATE_LIMIT: u32 = 1 << 24;

/// How many ancestor popups the walk to a popup's root follows before
/// giving up (and leaving the popup unconstrained). Real menus nest a handful
/// deep; the bound is what keeps a parent chain a client managed to close
/// into a cycle from hanging the compositor here.
const MAX_POPUP_DEPTH: usize = 64;

/// `positioner`'s geometry adjusted into `target` (both in the parent's
/// window-geometry coordinates), or `None` when any input is beyond
/// [`COORDINATE_LIMIT`] and the arithmetic cannot be trusted to stay in
/// range.
pub(super) fn constrain(
    positioner: PositionerState,
    target: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    let inputs = [
        positioner.rect_size.w,
        positioner.rect_size.h,
        positioner.anchor_rect.loc.x,
        positioner.anchor_rect.loc.y,
        positioner.anchor_rect.size.w,
        positioner.anchor_rect.size.h,
        positioner.offset.x,
        positioner.offset.y,
        target.loc.x,
        target.loc.y,
        target.size.w,
        target.size.h,
    ];
    if inputs.iter().any(|v| v.unsigned_abs() > COORDINATE_LIMIT) {
        return None;
    }
    Some(positioner.get_unconstrained_geometry(target))
}

impl State {
    /// The geometry `popup` should be configured with for `positioner`:
    /// constrained against its target (see the module doc), or `None` when
    /// no target is known and the positioner's own geometry stands.
    pub(super) fn constrained_popup_geometry(
        &self,
        popup: &PopupSurface,
        positioner: PositionerState,
    ) -> Option<Rectangle<i32, Logical>> {
        constrain(positioner, self.popup_constraint_target(popup)?)
    }

    /// Applies [`State::constrained_popup_geometry`] to `popup`'s pending
    /// state, ahead of its initial configure. A popup with no known target
    /// keeps the geometry Smithay already put there (the positioner's own).
    ///
    /// The positioner is read out, and the target computed, *outside* the
    /// pending-state closure: that closure holds this surface's state lock,
    /// and the target walk takes other surfaces' locks.
    pub(super) fn constrain_popup_before_initial_configure(&self, popup: &PopupSurface) {
        let positioner = popup.with_pending_state(|state| state.positioner);
        if let Some(geometry) = self.constrained_popup_geometry(popup, positioner) {
            popup.with_pending_state(|state| state.geometry = geometry);
        }
    }

    /// The target rectangle for `popup`, in its immediate parent's
    /// window-geometry coordinates.
    fn popup_constraint_target(&self, popup: &PopupSurface) -> Option<Rectangle<i32, Logical>> {
        let (root, offset) = popup_root_and_offset(popup.get_parent_surface()?)?;
        // Target in the root's coordinates, then shifted by the immediate
        // parent's offset from the root. Saturating throughout: the offset
        // sums client-chosen positions, and a saturated coordinate lands
        // beyond `COORDINATE_LIMIT`, which `constrain` refuses.
        let in_root = self
            .window_popup_target(&root)
            .or_else(|| self.layer_popup_target(&root))?;
        Some(Rectangle::new(
            Point::new(
                in_root.loc.x.saturating_sub(offset.x),
                in_root.loc.y.saturating_sub(offset.y),
            ),
            in_root.size,
        ))
    }

    /// A window root's target, relative to its window geometry origin: its
    /// output's usable area, or the whole output while this window covers it
    /// fullscreen. `None` for a surface that is not a window, or a window on
    /// no output.
    fn window_popup_target(&self, root: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        let id = self.id_of(root)?;
        let window = self.window(id)?;
        let output_id = output_clip::placed_on(window)?;
        let whole = self.space.output_geometry(self.outputs.get(output_id)?)?;
        let area = if self.world.fullscreen_on(output_id) == Some(id) {
            whole
        } else {
            self.world
                .usable_area(output_id)
                .filter(|usable| usable.w > 0 && usable.h > 0)
                .map(|usable| {
                    Rectangle::new((usable.x, usable.y).into(), (usable.w, usable.h).into())
                })
                .unwrap_or(whole)
        };
        // The window geometry origin in global coordinates: `apply()` maps
        // each window with its geometry origin at the placement position,
        // and the render path draws it from exactly this lookup.
        let origin = self.space.element_location(window)?;
        Some(Rectangle::new(
            Point::new(
                area.loc.x.saturating_sub(origin.x),
                area.loc.y.saturating_sub(origin.y),
            ),
            area.size,
        ))
    }

    /// A layer-surface root's target, relative to the layer surface's own
    /// origin (a layer surface has no window geometry of its own): its whole
    /// output. `None` for a surface on no output's layer map.
    fn layer_popup_target(&self, root: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        let output = self.output_of_layer(root)?;
        let size = self.space.output_geometry(&output)?.size;
        // One guard, dropped at the end of the statement: `output_of_layer`
        // has released its own by now (see its doc on why the two must not
        // overlap).
        let layer_loc = {
            let layers = layer_map_for_output(&output);
            let layer = layers.layer_for_surface(root, WindowSurfaceType::TOPLEVEL)?;
            layers.layer_geometry(layer)?.loc
        };
        Some(Rectangle::new(
            Point::new(layer_loc.x.saturating_neg(), layer_loc.y.saturating_neg()),
            size,
        ))
    }
}

/// Walks from a popup's immediate `parent` up through every ancestor popup to
/// the root surface (a window or a layer surface), summing each ancestor
/// popup's committed position: the parent's window-geometry origin relative
/// to the root's.
///
/// `None` when an ancestor popup has no parent -- which a client can arrange
/// (a popup created parentless for a layer surface that never adopted it,
/// then a submenu of it) -- or its role data is gone. Not
/// `get_popup_toplevel_coords`, which unwraps that parent.
///
/// Reads each ancestor's state directly (one `with_states` per level) rather
/// than through `PopupManager::find_popup`, which scans every tracked popup
/// per call: the walk is O(depth), not O(depth x popups).
fn popup_root_and_offset(mut parent: WlSurface) -> Option<(WlSurface, Point<i32, Logical>)> {
    let mut offset = Point::<i32, Logical>::default();
    let mut depth = 0;
    while get_role(&parent) == Some(XDG_POPUP_ROLE) {
        depth += 1;
        if depth > MAX_POPUP_DEPTH {
            return None;
        }
        let (loc, next) = with_states(&parent, |states| {
            let next = states
                .data_map
                .get::<XdgPopupSurfaceData>()?
                .lock()
                .ok()?
                .parent
                .clone();
            let loc = states
                .cached_state
                .get::<PopupCachedState>()
                .current()
                .last_acked
                .as_ref()
                .map(|configure| configure.state.geometry.loc)
                .unwrap_or_default();
            Some((loc, next))
        })?;
        offset = Point::new(
            offset.x.saturating_add(loc.x),
            offset.y.saturating_add(loc.y),
        );
        parent = next?;
    }
    Some((parent, offset))
}
