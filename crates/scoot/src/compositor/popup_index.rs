//! Every live `xdg_popup` by its surface, so its first commit finds it
//! without a scan.
//!
//! The first commit of every popup earns it an initial configure
//! (`send_popup_initial_configure` in `handlers.rs`), and that used to reach
//! it through `PopupManager::find_popup` -- which walks every tree into a
//! fresh `Vec` on every call. One call is O(every live popup) plus an
//! allocation of that size; a burst that tracks-then-commits N popups pays
//! it N times, O(N^2) with N growing allocations. Connections multiply it:
//! K connections at M popups each commit K*M popups against a global tree
//! of K*M, with no client past its cap -- measured 3.9 s of a 4.1 s stall
//! at 40 x 128 (`scripts/popup-flood/run-many.sh`, dev VM release).
//!
//! So every tracked `xdg_popup` is filed here under its surface's id, and
//! the configure path reads it back in O(1) with no scan and no `Vec`.
//! Same-client parenting is all the protocol allows (a client names only
//! its own objects as a popup's parent), so each popup tree holds at most
//! its owner's popups plus a few input-method leaves -- the per-tree scans
//! (`PopupTree::insert`, `popups_for_surface`) were already bounded by the
//! per-client cap, and only this global one needed replacing.
//!
//! ## Exactness: every write site, so a later change cannot conflate the
//! index with something else
//!
//! - **Inserted in `XdgShellHandler::new_popup`** (`handlers.rs`), once
//!   `PopupManager::track_popup` has taken the popup. A popup refused by
//!   `popup_parent::admit` never reaches tracking, and a tracking Smithay
//!   refuses (`DeadResource`) is never filed, so neither leaks an entry.
//!   Re-making a popup over its old surface overwrites: the old popup's
//!   destroy removed it before the new creation filed afresh (see
//!   `Admission::Reused`).
//! - **Removed in `popup_parent::popup_destroyed`**, but only when the
//!   dying popup still owns its surface's record -- the same ownership
//!   check that releases the per-client count, so a refused popup (which
//!   never owned one) and a reincarnated surface's old popup (already
//!   released) can never double-remove. Smithay reaches that hook for every
//!   `xdg_popup` that goes away, an explicit destroy and a disconnect
//!   alike, so a killed client's entries drain with its teardown.
//! - **A dismissal is not a destroy, and the index knows it.** A refused
//!   grab dismisses the popup out of its tree while the object lives on
//!   (Smithay-side `ungrab` paths do the same, invisibly), so an entry can
//!   outlive its node's membership. That is why the configure lookup pairs
//!   the index with a membership check over the popup's own tree (see
//!   `send_popup_initial_configure`): filed-but-dismissed is never
//!   configured, exactly as the old walk missing it never configured it.
//!   The entry itself is still removed once, at destroy.
//! - **Keyed by the surface's `ObjectId`**, which compares the client id
//!   and generation serial as well as the bare id (wayland-backend 0.3.17
//!   `rs/server_impl/mod.rs`), so a stale entry could never equal a live
//!   surface even if one survived.
//! - **Input-method popups are not filed.** They are tracked directly
//!   (`input_method.rs`), never through `new_popup`, and they carry
//!   `zwp_input_popup_surface_v2`, not `xdg_popup` -- the configure path's
//!   role check keeps them out before any lookup, exactly as the
//!   `find_popup` version did.
//!
//! ## Cost
//!
//! One map insert per popup tracked, one remove per popup destroyed, one
//! lookup per commit of a popup-role surface -- on popup paths, never per
//! frame. The lookup clones the filed handle on a hit, which is what
//! `find_popup`'s `.cloned()` did, minus the tree walk and the `Vec`. A
//! surface that is not a popup never reaches the lookup (the role check
//! first), so ordinary commits pay one role check, as before.
//!
//! ## What this leaves (honest residual, not this file)
//!
//! Two Smithay-side scans stay linear-per-op with small constants, both
//! verified against the pinned rev's source: `PopupManager::commit`'s
//! `position` scan of the unmapped list on every surface commit, and the
//! `xdg_popup` destructor's linear `known_popups` search. Measured at
//! 40 x 128 (dev VM release): ~80 ms track, ~80 ms destroy. A further
//! index would be a Smithay fork change (`docs/forks.md`); with the
//! configure walk gone there is no seconds-scale stall left to justify
//! one.

use std::collections::HashMap;

use smithay::desktop::PopupKind;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Every live `xdg_popup` filed at tracking, by its surface's id: what
/// the initial configure's lookup reads, paired there with a membership
/// check over the popup's own tree (a dismissal removes the node while
/// the object lives on -- see the module doc).
#[derive(Debug, Default)]
pub struct PopupIndex {
    /// The live tracked popups by surface id.
    live: HashMap<ObjectId, PopupKind>,
}

impl PopupIndex {
    /// Files `kind`'s popup under its surface. Overwrites: re-making a
    /// popup over its old surface files the new one where the old was,
    /// after the old one's destroy removed it.
    pub(super) fn insert(&mut self, kind: &PopupKind) {
        self.live.insert(kind.wl_surface().id(), kind.clone());
    }

    /// The live tracked popup on `surface`, if there is one. A surface
    /// that is not a popup, or whose popup was never tracked, reads back
    /// `None` -- the caller treats that the way `find_popup`'s miss was
    /// treated (the popup went away between checks; nothing to configure).
    pub(super) fn get(&self, surface: &WlSurface) -> Option<PopupKind> {
        self.live.get(&surface.id()).cloned()
    }

    /// Forgets `surface`'s popup. Idempotent by the caller's record: only
    /// the popup that owns the surface's record reaches this, once.
    pub(super) fn remove(&mut self, surface: &WlSurface) {
        self.live.remove(&surface.id());
    }

    /// How many popups are filed. Test-only: the drift pin beside the
    /// per-client count's own.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.live.len()
    }
}
