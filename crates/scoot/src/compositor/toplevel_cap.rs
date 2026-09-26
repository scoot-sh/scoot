//! How many live `xdg_toplevel`s one client may have: at most
//! [`MAX_TOPLEVELS_PER_CLIENT`].
//!
//! Every toplevel enters the core, and the core's costs scale with the
//! count: `World::arrange` runs per frame per output and per `apply()`
//! (multi-millisecond at a few thousand windows). The float paths no longer
//! run an arrangement per window (single-window queries, one arrangement
//! per re-centre -- see `docs/backlog/core/per-client-toplevel-cap.md`),
//! but the per-frame arrangement still scales with the count, so a client
//! that opens toplevels without bound stalls every frame for everyone. So each
//! client gets a bound, and a toplevel past it is refused the way the other
//! per-client bounds refuse abuse: the client is disconnected.
//!
//! ## The number
//!
//! [`MAX_TOPLEVELS_PER_CLIENT`] is 128. Real clients open a handful; a
//! browser restoring a session opens dozens. 128 is several times the
//! heaviest legitimate use and an order of magnitude below the measured
//! pain (about a thousand windows, where one arrangement costs roughly a
//! millisecond in release): one client's worst case stays a fraction of a
//! frame. It is generous on purpose, because tripping it disconnects the
//! client.
//!
//! ## Refusal form
//!
//! A toplevel past the cap disconnects its client with
//! `wl_display.error(no_memory)` (see `no_memory.rs`), like an `add` past
//! the pending-plane cap and an acquire past the syncobj-wait cap. The
//! `xdg_toplevel` interface has no error for "too many", and this is not a
//! malformed request. Kept-out-of-the-core was the alternative, and it is
//! the worse one: the toplevel would exist with no configure ever sent for
//! it, a silent blackhole the client waits on while its objects leak --
//! where a kill is loud, logged, and frees everything at once.
//!
//! ## How it is counted
//!
//! One unit per live claimed `xdg_toplevel`, held in two maps (the shape
//! `dmabuf/pending_planes.rs` uses for its per-params and per-client
//! counts): `owner` names which client each claimed window id belongs to,
//! `per_client` sums them. Every read and write site, so a later change
//! cannot conflate the count with something else:
//!
//! - **Claimed in `State::add_window`**, the one path every `xdg_toplevel`
//!   enters through (`XdgShellHandler::new_toplevel`), before the window
//!   reaches the core or any other list. A refused toplevel consumes one
//!   `next_id` and nothing else: the id sequence stays monotonic (never
//!   reused, which `foreign_toplevel.rs` relies on), and the refusal leaves
//!   no window for `toplevel_destroyed` to find.
//! - **Released in `State::remove_window`**, which
//!   `XdgShellHandler::toplevel_destroyed` reaches for every toplevel that
//!   goes away -- an explicit destroy and a disconnect alike, since the
//!   backend destroys every object of a dying client. Idempotent by the
//!   `owner` map: a refused toplevel was never claimed, and an X window
//!   never is (see below), so neither releases anything.
//! - **XWayland is not counted.** Managed X windows enter through
//!   `map_x11_window`, which bypasses `add_window`, so they neither claim
//!   nor release. That is a separate subsystem with its own mapping path,
//!   and bounding it is filed separately
//!   (`docs/backlog/core/xwayland-toplevel-cap.md`).
//! - **A toplevel with no client is admitted uncounted.** Creation always
//!   has one in practice; if it somehow has not, there is nothing to charge
//!   and nothing that could release it, so failing open keeps the maps exact.
//!
//! ## Cost
//!
//! One map lookup and one insert per toplevel created, one lookup and one
//! remove per toplevel destroyed -- on window-open paths, never per frame
//! or per commit.

use std::collections::HashMap;

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::wayland::shell::xdg::ToplevelSurface;

use scoot_core::WindowId;

use super::State;

#[cfg(test)]
mod tests;

/// How many live `xdg_toplevel`s one client may have, across all of them.
/// See the module doc. Past it the toplevel is refused by disconnecting the
/// client with `wl_display.error(no_memory)`.
pub(super) const MAX_TOPLEVELS_PER_CLIENT: u32 = 128;

/// The live claimed toplevels, per client and per window. See the module
/// doc. An entry exists in `per_client` only while it is nonzero, and in
/// `owner` only while the window is claimed, so both maps are bounded by
/// live toplevels.
#[derive(Debug, Default)]
pub struct ToplevelCap {
    /// Claimed toplevels per client: what the bound reads.
    per_client: HashMap<ClientId, u32>,
    /// Which client each claimed window id belongs to: what the release
    /// reads, so a destroy never touches another client's count -- not even
    /// on teardown, when the backend has already forgotten the client and
    /// the count can no longer be re-derived from the surface.
    owner: HashMap<WindowId, ClientId>,
}

impl ToplevelCap {
    /// How many live toplevels `client` has claimed.
    #[cfg(test)]
    pub(super) fn live_for(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).copied().unwrap_or(0)
    }

    /// How many toplevels every client holds between them. Test-only.
    #[cfg(test)]
    pub(super) fn in_flight(&self) -> u32 {
        self.per_client.values().sum()
    }

    /// Claims one toplevel `id` for `client`, unless it already holds
    /// [`MAX_TOPLEVELS_PER_CLIENT`], in which case nothing is counted and
    /// how many it holds is handed back.
    ///
    /// The counts cannot overflow: each unit is a live toplevel object, and
    /// the cap stops a client far below `u32::MAX`.
    fn try_claim(&mut self, client: &ClientId, id: WindowId) -> Result<(), u32> {
        let live = self.per_client.entry(client.clone()).or_insert(0);
        if *live >= MAX_TOPLEVELS_PER_CLIENT {
            return Err(*live);
        }
        *live += 1;
        // `add_window` never claims an id twice (each claim mints a fresh
        // `next_id`), so this insert never overwrites.
        self.owner.insert(id, client.clone());
        Ok(())
    }

    /// Forgets the claim on `id`, if it has one. Idempotent: a refused
    /// toplevel and an X window were never claimed, and a claimed one is
    /// forgotten once.
    pub(super) fn release(&mut self, id: &WindowId) {
        let Some(client) = self.owner.remove(id) else {
            return;
        };
        if let Some(live) = self.per_client.get_mut(&client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.per_client.remove(&client);
            }
        }
    }
}

impl State {
    /// Claims `surface`'s toplevel against its client's cap, for a window
    /// about to enter the core. `false` when the client is past
    /// [`MAX_TOPLEVELS_PER_CLIENT`]: it has been disconnected with
    /// `wl_display.error(no_memory)`, and the caller must not admit the
    /// window anywhere.
    pub(super) fn claim_toplevel(&mut self, surface: &ToplevelSurface, id: WindowId) -> bool {
        let Some(client) = surface.wl_surface().client() else {
            return true;
        };
        match self.toplevel_cap.try_claim(&client.id(), id) {
            Ok(()) => true,
            Err(live) => {
                tracing::warn!(
                    live,
                    cap = MAX_TOPLEVELS_PER_CLIENT,
                    "refusing an xdg_toplevel past the per-client cap; disconnecting the client"
                );
                super::no_memory::disconnect(
                    &self.display_handle,
                    &client,
                    format!(
                        "at most {MAX_TOPLEVELS_PER_CLIENT} live xdg_toplevels per client, \
                         and this client holds {live}"
                    ),
                );
                false
            }
        }
    }
}
