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
//! - **XWayland is counted separately, per X client.** Managed X windows
//!   enter through `map_x11_window`, which bypasses `add_window`, so they
//!   never touch the claim above. They hold [`X11ToplevelCap`] units instead
//!   -- one per live managed window, charged to the window id's client bits
//!   (see `xwayland/focus.rs`'s `x_client_key`), claimed at the map request
//!   and released in `remove_window` beside the claim above. A window past
//!   its client's bound is refused the map, like the insane-frame-extents
//!   refusal already in `map_x11_window`: X has no client object to post an
//!   error to, so there is no kill to send. Override-redirect windows are
//!   not counted: they never enter the core (`x11_unmanaged`), so they carry
//!   none of the arrangement cost this bounds.
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

// ---------------------------------------------------------------------------
// The XWayland half: one bound per X client, refused by refusing the map.
// ---------------------------------------------------------------------------

/// How many live managed X windows one X client may have, across all of
/// them. Past it the window is refused its map (see below).
///
/// Deliberately the xdg cap's number, not a second decision: one X window
/// costs the core the same per-frame arrangement as one `xdg_toplevel`, so
/// the same worst case per client deserves the same bound. Kept a separate
/// constant so changing one never silently changes the other.
///
/// An X client is its window ids' client bits (see `xwayland/focus.rs`'s
/// `x_client_key`) -- the server's word on which connection created the
/// window -- not a Wayland `ClientId`. The two identities are not unified:
/// there is no Wayland object to charge an X window to (XWayland's own
/// client stands for every X client at once; killing it would take every X
/// window with it), and no X notion to charge an `xdg_toplevel` to.
///
/// Only in `xwayland` builds: a default build carries no X code at all.
#[cfg(feature = "xwayland")]
pub(super) const MAX_X11_TOPLEVELS_PER_CLIENT: u32 = 128;

/// The live managed X windows, per X client and per window: the X half of
/// [`ToplevelCap`], in the same two-map shape and the same counting
/// discipline. An entry exists in `per_client` only while it is nonzero,
/// and in `owner` only while the window is claimed, so both maps are
/// bounded by live managed X windows.
///
/// Only in `xwayland` builds, with the counter's users.
#[cfg(feature = "xwayland")]
#[derive(Debug, Default)]
pub struct X11ToplevelCap {
    /// Claimed windows per X client, keyed by window-id client bits: what
    /// the bound reads.
    per_client: HashMap<u32, u32>,
    /// Which X client each claimed core window id belongs to: what the
    /// release reads, so an unmap never touches another client's count --
    /// not even on teardown, when the X window the id came from is gone and
    /// the count can no longer be re-derived from it.
    owner: HashMap<WindowId, u32>,
}

#[cfg(feature = "xwayland")]
impl X11ToplevelCap {
    /// How many live managed windows `client` (window-id client bits) has
    /// claimed.
    fn live(&self, client: &u32) -> u32 {
        self.per_client.get(client).copied().unwrap_or(0)
    }

    /// How many live managed windows `client` has claimed. Read by the
    /// refusal log line as well as the tests, so -- unlike the xdg cap's
    /// accessor -- not test-gated.
    pub(super) fn live_for(&self, client: &u32) -> u32 {
        self.live(client)
    }

    /// How many managed X windows every X client holds between them.
    /// Test-only.
    #[cfg(test)]
    pub(super) fn in_flight(&self) -> u32 {
        self.per_client.values().sum()
    }

    /// Whether `client` may map one more window: `false` once it holds
    /// [`MAX_X11_TOPLEVELS_PER_CLIENT`]. A read, so `map_x11_window` can
    /// refuse the map before anything is granted; the claim itself is
    /// recorded by [`X11ToplevelCap::claim`] once the window is in.
    ///
    /// The check and the claim are two steps rather than one `try_claim`
    /// because the map grants first (`set_mapped`) and mints the core id
    /// after: claiming before the grant would leak a unit when the grant
    /// fails, and claiming-then-unmapping on a full count would map and
    /// unmap a window the client sees flicker. Sound because both run on
    /// the loop, with no dispatch between them.
    pub(super) fn admits(&self, client: &u32) -> bool {
        self.live(client) < MAX_X11_TOPLEVELS_PER_CLIENT
    }

    /// Records the claim on core window `id` for `client`. Call only after
    /// [`X11ToplevelCap::admits`] said yes on the same dispatch: the count
    /// cannot overflow (each unit is a live managed window, and the cap
    /// stops a client far below `u32::MAX`), and `map_x11_window` never
    /// claims an id twice (each claim mints a fresh `next_id`), so this
    /// insert never overwrites.
    pub(super) fn claim(&mut self, client: u32, id: WindowId) {
        *self.per_client.entry(client).or_insert(0) += 1;
        self.owner.insert(id, client);
    }

    /// Forgets the claim on `id`, if it has one. Idempotent: an
    /// `xdg_toplevel` was never claimed here, a refused X window never
    /// reached [`X11ToplevelCap::claim`], and a claimed one is forgotten
    /// once, wherever `remove_window` runs for it -- an unmap, a destroy, a
    /// server death, or the frame-extents withdrawal.
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
