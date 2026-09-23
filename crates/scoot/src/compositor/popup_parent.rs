//! An `xdg_popup`'s parent chain: the one step up it that every walk here
//! takes, and the rule that keeps every walk up it finite.
//!
//! # A parent chain never loops
//!
//! A popup names its parent `xdg_surface` at `xdg_surface.get_popup`, and
//! nothing at the pinned Smithay rev checks that parent against the popup
//! being created. A client can therefore close the chain into a loop: name
//! the popup's *own* `xdg_surface` as its parent, or make a popup `A` of a
//! bare, role-less `xdg_surface` `X` and then turn `X` into a popup whose
//! parent is `A`. Every walk up the chain then runs forever -- and the first
//! one runs straight away: `PopupManager::track_popup`, in `new_popup`,
//! calls Smithay's `find_popup_root_surface`, which loops until it reaches a
//! non-popup surface. Measured: one such request froze the compositor, every
//! client with it (the harness test timed out and was killed).
//!
//! So `new_popup` refuses a popup whose chain loops *before* tracking it
//! ([`closes_a_cycle`]), clears the refused popup's parent so the loop is
//! gone from the data too, and disconnects the client. That check is exact,
//! and it terminates, by induction: a popup's parent is set only at
//! `get_popup` -- checked here, for every popup -- or by
//! `zwlr_layer_surface_v1.get_popup`, which always sets a layer surface, and
//! a layer surface is not a popup, so it ends a chain rather than extending
//! one. Every chain that existed before this popup was therefore finite, and
//! a loop that appears now must pass through the popup being created. Every
//! other walk up a chain -- Smithay's, and `popup_constraint.rs`'s -- then
//! terminates by the same invariant.

use std::sync::PoisonError;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{get_role, with_states};
use smithay::wayland::shell::xdg::{
    PopupCachedState, PopupSurface, XDG_POPUP_ROLE, XdgPopupSurfaceData,
};

/// One step up the chain from a popup-role `surface`: its parent (`None`
/// when it has none yet -- created parentless for a layer surface that has
/// not adopted it) and its committed position (as last acked and committed,
/// the position the render tree places it by; the origin before its first
/// configured commit).
///
/// `None` for a surface that is not a popup -- the root of a chain -- or
/// whose popup role data is gone.
pub(super) fn popup_link(surface: &WlSurface) -> Option<(Option<WlSurface>, Point<i32, Logical>)> {
    if get_role(surface) != Some(XDG_POPUP_ROLE) {
        return None;
    }
    with_states(surface, |states| {
        let parent = states
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
        Some((parent, loc))
    })
}

/// Whether walking up from `popup`'s parent comes back to `popup` itself --
/// the only way a chain can loop (see the module doc).
fn closes_a_cycle(popup: &WlSurface) -> bool {
    let mut next = popup_link(popup).and_then(|(parent, _)| parent);
    while let Some(surface) = next {
        if surface == *popup {
            return true;
        }
        next = popup_link(&surface).and_then(|(parent, _)| parent);
    }
    false
}

/// Refuses `popup` if its parent chain loops, disconnecting its client, and
/// says whether it did. `new_popup` calls this before tracking the popup,
/// because tracking is itself the first walk up the chain.
///
/// The protocol's error for this is `xdg_wm_base.invalid_popup_parent`, but
/// the pinned rev keeps the `xdg_wm_base` resource behind a `pub(crate)`
/// field, so the error is posted on the popup itself, with that error's code
/// and a message naming it. Either way the client is disconnected, which is
/// what matters; logged at `warn` like the popup-grab refusals, so the
/// disconnect is diagnosable.
///
/// The loop is also broken in the data, not just refused: Smithay wrote the
/// popup's role data, parent included, before `new_popup` ran, so without
/// this the loop would outlive the refusal until the client's teardown
/// resets it. Nothing can walk it through a request in that window -- once
/// killed, a client's requests are no longer read (wayland-backend 0.3.17,
/// `rs/server_impl/client.rs`, `next_request` answers `EPIPE` for a killed
/// client), which the same-flush `reposition` in this module's tests
/// confirms -- but the teardown itself runs destructors in no stated
/// order. Every loop runs through this popup, so clearing its parent breaks
/// all of them, and the module doc's "no chain loops" holds from here on
/// rather than from whenever teardown gets to it.
pub(super) fn refuse_if_cyclic(popup: &PopupSurface) -> bool {
    if !closes_a_cycle(popup.wl_surface()) {
        return false;
    }
    with_states(popup.wl_surface(), |states| {
        if let Some(data) = states.data_map.get::<XdgPopupSurfaceData>() {
            data.lock().unwrap_or_else(PoisonError::into_inner).parent = None;
        }
    });
    tracing::warn!(
        client = ?popup.wl_surface().client().map(|client| client.id()),
        "xdg_popup's parent chain loops back to itself; disconnecting the client"
    );
    popup.xdg_popup().post_error(
        xdg_wm_base::Error::InvalidPopupParent as u32,
        "invalid_popup_parent: this popup's parent chain loops back to the popup itself",
    );
    true
}
