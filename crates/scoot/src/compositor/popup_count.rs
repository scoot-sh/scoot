//! How many live `xdg_popup`s one client may have: at most
//! [`MAX_POPUPS_PER_CLIENT`].
//!
//! Every popup joins a `PopupTree`, and the tree's costs scale with how many
//! popups share it: `PopupTree::insert` searches the whole tree for the new
//! popup's parent on every popup tracked, and `PopupManager::find_popup` --
//! which the first commit of every popup reaches through
//! `send_popup_initial_configure` -- walks every tree into a fresh `Vec`.
//! Side by side (all within the depth cap, so the depth bound cannot help)
//! that is O(n) per popup, O(n^2) for the burst: about 2000 popups stalled
//! the compositor for the better part of a second in release, about 5000
//! for over five (see `docs/backlog/core/popup-count-quadratic.md`). So each
//! client gets a bound, and a popup past it is refused the way the other
//! per-client bounds refuse abuse: the client is disconnected.
//!
//! ## The number
//!
//! [`MAX_POPUPS_PER_CLIENT`] is 128. Real clients open a handful of popups
//! -- a menu, a submenu or two, a dialog's dropdown -- so 128 is an order of
//! magnitude above anything legitimate, and it keeps the worst case small:
//! at the bound the whole burst costs on the order of 128^2 tree steps, a
//! few milliseconds, a fraction of a frame. It is generous on purpose,
//! because tripping it disconnects the client.
//!
//! ## Refusal form
//!
//! A popup past the cap disconnects its client with
//! `wl_display.error(no_memory)` (see `no_memory.rs`), like an `xdg_toplevel`
//! past the toplevel cap and an `add` past the pending-plane cap. The
//! `xdg_popup` interface has no error for "too many", and this is not a
//! malformed request. Kept-out-of-the-tree was the alternative, and it is
//! the worse one: the popup would exist with no configure ever sent for it,
//! a silent blackhole the client waits on while its objects leak -- where a
//! kill is loud, logged, and frees everything at once.
//!
//! ## How it is counted
//!
//! One unit per live claimed `xdg_popup`, held in one map: `per_client` sums
//! them. The other half of the toplevel cap's two maps -- which client each
//! window belongs to -- lives in the popup's own `PopupRecord` instead
//! (`popup_parent.rs`): the surface's record already outlives the popup's
//! role object and is only touched when the dying popup still owns it, so
//! the charged client is read back exactly where the release happens. Every
//! read and write site, so a later change cannot conflate the count with
//! something else:
//!
//! - **Claimed in `popup_parent::admit`**, the one path every `xdg_popup`
//!   enters through (`XdgShellHandler::new_popup`), after the depth, loop
//!   and parent-liveness checks and before the parent's `live_children` is
//!   incremented. A popup refused by any of those checks never holds a
//!   claim, and a popup refused by this one never reaches the increment, so
//!   a refusal leaks nothing in either direction.
//! - **Released in `popup_parent::popup_destroyed`**, which Smithay reaches
//!   for every `xdg_popup` that goes away -- an explicit destroy and a
//!   disconnect alike, since the backend destroys every object of a dying
//!   client -- but only when the dying popup still owns its surface's
//!   record. A refused popup never owned one, and a reincarnated surface's
//!   old popup released its claim when it died, before the new one claimed
//!   afresh.
//! - **A popup with no client is admitted uncounted.** Creation always has
//!   one in practice; if it somehow has not, there is nothing to charge and
//!   nothing that could release it, so failing open keeps the map exact.
//! - **Input-method popups are not counted.** They cannot be anyone's parent
//!   (see `popup_parent.rs`), so they cannot deepen anything, and they are
//!   bounded in practice by the seats and text inputs they hang off -- the
//!   same scope as the depth cap, which counts only `xdg_popup`s too.
//! - **XWayland popups are not counted.** Override-redirect menus are drawn
//!   unmanaged and never enter the `PopupManager`; managed X windows never
//!   become `xdg_popup`s (they hold the per-X-client toplevel cap instead --
//!   see `toplevel_cap.rs`). The unmanaged ones hold that file's
//!   per-X-client unmanaged-window cap instead.
//!
//! ## Cost
//!
//! One map lookup and one insert per popup created, one lookup and one
//! remove per popup destroyed -- on popup-open paths, never per frame or
//! per commit. In particular the fix adds no allocation to any per-commit
//! path: `find_popup`'s per-commit tree walk and its `Vec` stay exactly as
//! they were, only bounded -- at most 128 popups deep, where before they
//! were unbounded.
//!
//! The scans themselves -- `PopupTree::insert` / `PopupNode::try_insert` and
//! `PopupManager::find_popup` live in Smithay at the pinned rev
//!     (`src/desktop/wayland/popup/manager.rs`); the `xdg_popup` destructor's
//!     linear search in `known_popups` too
//!     (`src/wayland/shell/xdg/handlers/surface/popup.rs`). All three stay
//!     as they are: with the count bounded, every one of them is bounded
//!     with it, and reworking Smithay's tree into an indexed lookup would be
//!     a fork change for a path that now costs milliseconds at worst.

use std::collections::HashMap;

use smithay::reexports::wayland_server::backend::ClientId;

/// How many live `xdg_popup`s one client may have, across all of them.
/// See the module doc. Past it the popup is refused by disconnecting the
/// client with `wl_display.error(no_memory)`.
pub(super) const MAX_POPUPS_PER_CLIENT: u32 = 128;

/// The live claimed popups per client: what the bound reads. An entry
/// exists only while it is nonzero, so the map is bounded by live popups.
///
/// The matching per-popup half -- which client each claimed popup was
/// charged to -- is the surface's own `PopupRecord` in `popup_parent.rs`,
/// read back at release.
#[derive(Debug, Default)]
pub struct PopupCount {
    /// Claimed popups per client: what the bound reads.
    per_client: HashMap<ClientId, u32>,
}

impl PopupCount {
    /// How many live popups `client` has claimed.
    #[cfg(test)]
    pub(super) fn live_for(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).copied().unwrap_or(0)
    }

    /// How many popups every client holds between them. Test-only.
    #[cfg(test)]
    pub(super) fn in_flight(&self) -> u32 {
        self.per_client.values().sum()
    }

    /// Claims one popup for `client`, unless it already holds
    /// [`MAX_POPUPS_PER_CLIENT`], in which case nothing is counted and how
    /// many it holds is handed back.
    ///
    /// The counts cannot overflow: each unit is a live popup object, and
    /// the cap stops a client far below `u32::MAX`.
    pub(super) fn try_claim(&mut self, client: &ClientId) -> Result<(), u32> {
        let live = self.per_client.entry(client.clone()).or_insert(0);
        if *live >= MAX_POPUPS_PER_CLIENT {
            return Err(*live);
        }
        *live += 1;
        Ok(())
    }

    /// Forgets one popup charged to `client`. Idempotent by the caller's
    /// record: a refused popup was never claimed, and a claimed one is
    /// forgotten once, when its `xdg_popup` dies.
    pub(super) fn release(&mut self, client: &ClientId) {
        if let Some(live) = self.per_client.get_mut(client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.per_client.remove(client);
            }
        }
    }
}
