//! One shared per-client budget for binding manager/list globals.
//!
//! Four globals have the same unbounded shape: each bind costs the
//! compositor one handle object per workspace or window, and every change
//! walks every bound list. `ext_workspace_manager_v1` (per bind: one handle
//! per *workspace*), `ext_foreign_toplevel_list_v1` and
//! `zwlr_foreign_toplevel_manager_v1` (per bind: one handle per *window* --
//! the worst multiplier, since one client can create windows without limit),
//! and `zwlr_output_manager_v1` (per bind: one head plus its modes --
//! bounded, the least dangerous of the four).
//!
//! The budget is **shared across all four**, not one cap per global: a
//! client spending a per-global allowance on the ext list and then again on
//! the wlr manager would spend the worst multiplier twice. It is **per
//! client, not global**: a greedy client must not deny a well-behaved one --
//! the lesson of the IPC connection cap's accepted tradeoff, which this
//! mechanism does not repeat.
//!
//! ## The number
//!
//! [`MAX_BINDS_PER_CLIENT`] is 8. The floor is what legitimate clients bind:
//! stock quickshell 0.3.1 binds `zwlr_foreign_toplevel_manager_v1` exactly
//! once per connection (measured on the wire), a workspace-capable shell adds
//! `ext_workspace_manager_v1` once, and a display page or `wlr-randr` binds
//! `zwlr_output_manager_v1` once and transiently. Ordinary toolkit clients
//! bind none of these. So steady-state legitimate use is at most one bind per
//! global -- four total -- and 8 is twice that, with the second half as
//! headroom for the transient below and for a shell that double-binds during
//! a reload.
//!
//! Sized against the worst multiplier, the worst case one connection can
//! force is all 8 binds on the two window-multiplier globals: `8 x windows`
//! handle objects per global, each carrying its 4-5 announcement events.
//! Against the measured 200-in-a-burst window count that is ~3,200 small
//! objects -- low single-digit megabytes, and a `wl_output` bind then walks
//! them at ~30ns per id comparison (`same_client_as`), i.e. ~0.1ms. A
//! tighter number would bound a few more small objects per abuser; the
//! failure mode of a false positive is a shell permanently missing its
//! taskbar or workspace list (a refused bind stays refused until binds are
//! released), so the margin errs generous.
//!
//! ## How it is counted
//!
//! Keyed by [`ClientId`](smithay::reexports::wayland_server::backend::ClientId)
//! -- the bind-time `Client` object, not a pid, which is unstable (zero in an
//! invisible PID namespace, reused after exit). Each counted bind stores its
//! [`ObjectId`](smithay::reexports::wayland_server::backend::ObjectId), and
//! release removes exactly that generation: wayland-backend mints a fresh
//! serial per created object and folds it into `ObjectId` equality
//! (0.3.17 `rs/server_impl/{mod.rs,client.rs}`), so a client that destroys a
//! manager and rebinds on the same numeric protocol id can never release --
//! or collide with -- the new bind. The existing `ext_workspace.rs` handle
//! lookup already leans on the same property.
//!
//! Claimed in each global's `bind`, before anything is announced; released
//! idempotently in two places: synchronously on the `stop` request (so a
//! stop-and-rebind in one batch succeeds even at the cap) and in the
//! `destroyed` hook (the path a bare destroy and every disconnect takes).
//! A bind refused here is answered with the protocol's own `finished` --
//! never a protocol error, which would kill a legitimate shell on a miscount
//! -- and finished is per-client by construction: only the overflowing client
//! is told, never an innocent one. The send is deferred to loop idle (see
//! [`RefusedBind`]): three of the four `finished` events are destructors, and
//! sending one inside `bind` pulls the object out from under wayland-backend's
//! bind epilogue, which panics the whole compositor.
//!
//! ## The dead-but-unpruned transient
//!
//! Wayland dispatch batches: every ready client's requests run before
//! wayland-backend's cleanup delivers the destruction callbacks a *disconnect*
//! queued (see the `ext-workspace-client-lookup-per-bind` record). A client
//! disconnecting at the cap while another binds in the same epoll batch
//! therefore briefly counts the dead client's binds too. That over-count is
//! fail-safe (it refuses one extra bind rather than leaking one) and
//! self-correcting (the queued `destroyed` releases at cleanup; a client that
//! retries after a round trip succeeds).
//!
//! Bare destroys do *not* share it, though an earlier draft of this paragraph
//! said they did. A destructor *request* (`ext_foreign_toplevel_list_v1`'s
//! `destroy` -- the only one the four capped globals define) runs its
//! `destroyed` hook inline, in the same dispatch (`common_poll.rs`'s request
//! arm calls `destroyed` right after `request` when the opcode is a
//! destructor), so destroy-and-rebind in one batch already sees the freed
//! slot. `stop` likewise releases synchronously in its handler rather than
//! waiting for the `finished` it sends to come back through cleanup. The
//! disconnect race above is the only shape left, and it is not reachable from
//! the test harness on purpose (disconnecting settles before anything else
//! runs) -- stated here so the analysis is on record rather than re-derived.
//!
//! ## The seam shm pools and screencopy sessions join later
//!
//! NOT built here -- this module is the table and the policy; the two halves
//! below are documented so they land as interception points, not as a fifth
//! mechanism:
//!
//! - **`wl_shm` pools** (`docs/backlog/resolved/shm-pool-count-cap-done.md`):
//!   the per-pool byte cap stays. What landed since is a *count*, not the
//!   byte total this seam sketched: `shm_pools.rs` caps live pools per
//!   client on the same `ClientId` key and the same claim-before-delegation /
//!   release-in-`destroyed` shape, as its own counter rather than a second
//!   column here -- pools refuse with a protocol error that kills, binds
//!   refuse with a `finished` that does not, and one table across those two
//!   refusal forms would be the awkward shoehorn this paragraph warned
//!   against. The byte total itself is still open, and needs an upstream
//!   size accessor first (see `shm_pools.rs` for the wall, stated with
//!   sources).
//! - **Screencopy sessions** (`docs/backlog/resolved/screencopy-session-cap-done.md`):
//!   the frames half stays where it is (per-client, pre-delegation, in
//!   `dispatch.rs`). Sessions join here the same way: claim on the manager
//!   request that creates one -- again visible with its `Client` at the
//!   blanket `request` seam even though Smithay's session handler exposes no
//!   client identity -- and release on session destroy. That record deferred
//!   sessions to "whatever shared mechanism closes the sibling entries";
//!   this is that mechanism, keyed the way sessions need (by client, released
//!   idempotently) so no new identity scheme is required when they land.

use std::collections::{HashMap, HashSet};

use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::{Client, Resource};

use super::State;

#[cfg(test)]
mod tests;

/// How many manager/list binds one Wayland client may hold across the four
/// globals in [`crate::compositor`] that have the binds-times-windows shape.
///
/// 8: twice the legitimate maximum (one bind per global -- see the module doc
/// for the measured floor), with the headroom covering the dead-but-unpruned
/// transient rather than legitimate growth. A client past it is answered with
/// the protocol's own `finished` on the excess bind, never a protocol error.
pub(super) const MAX_BINDS_PER_CLIENT: u32 = 8;

/// The per-client bind budget. See the module doc for the policy; the only
/// writer is each capped global's `bind`, the only releasers its `stop` and
/// `destroyed`.
#[derive(Debug, Default)]
pub struct BindBudget {
    /// The live counted binds per client. An entry exists only while the
    /// client holds at least one -- the empty set is removed on release --
    /// so this is bounded by live protocol objects the same way
    /// wayland-backend already bounds them, and a counter that only grows
    /// (the same leak in a new place) is impossible by construction.
    held: HashMap<ClientId, HashSet<ObjectId>>,
    /// Binds refused for being over budget, whose `finished` has not been
    /// sent yet. Drained by [`State::defer_bind_refusal`]'s idle callback;
    /// see [`RefusedBind`] for why the send cannot happen in `bind`.
    refused: Vec<RefusedBind>,
}

impl BindBudget {
    /// Counts one more live bind for `client`, refusing past
    /// [`MAX_BINDS_PER_CLIENT`].
    ///
    /// Returns whether the caller must refuse the bind -- and the refusal
    /// itself (the protocol's own `finished` on the fresh object) stays with
    /// the caller, which holds the typed object this module never sees.
    /// Counting and refusing split that way for the same reason the capture
    /// frame guard keeps both halves together does not apply here: the
    /// counted key (the client) and the error target (the fresh object) are
    /// different objects.
    ///
    /// Called *before* anything is announced, so a refused bind never
    /// allocates and a counted one is always registered: every caller pairs
    /// this with [`Self::release_bind`] on its announce-failure paths, which
    /// is what keeps the count exact rather than leaking on a path that
    /// counts but never creates.
    pub(super) fn refuse_bind(&mut self, client: &Client, id: &ObjectId) -> bool {
        let held = self.held.entry(client.id()).or_default();
        if held.len() >= MAX_BINDS_PER_CLIENT as usize {
            tracing::debug!(
                held = held.len(),
                max = MAX_BINDS_PER_CLIENT,
                "refusing a manager/list bind past the per-client budget (finished, not a protocol error)"
            );
            return true;
        }
        held.insert(id.clone());
        false
    }

    /// Forgets one counted bind: the idempotent half of
    /// [`Self::refuse_bind`].
    ///
    /// Called from the `stop` request (synchronous, so a stop-and-rebind in
    /// one batch succeeds) and from the `destroyed` hook (the path a bare
    /// destroy and every disconnect takes). Removing by exact [`ObjectId`] --
    /// serial included -- is what makes calling both safe, and calling either
    /// twice harmless: the second call finds nothing and does nothing.
    /// A release for an id never counted (a bookkeeping bug, not a client
    /// one) is likewise a no-op rather than a panic.
    pub(super) fn release_bind(&mut self, client: &ClientId, id: &ObjectId) {
        if let Some(held) = self.held.get_mut(client) {
            held.remove(id);
            if held.is_empty() {
                self.held.remove(client);
            }
        }
    }

    /// How many binds `client` currently holds. Test-only: the budget tests
    /// assert the bookkeeping drains, which a compositor-side count states
    /// directly.
    #[cfg(test)]
    pub(super) fn count(&self, client: &ClientId) -> usize {
        self.held.get(client).map(HashSet::len).unwrap_or(0)
    }

    /// Retracts a deferred refusal whose object was stopped before the idle
    /// callback ran, so it is not finished twice.
    ///
    /// This cannot suppress a *legitimate* `finished`, and the three clauses
    /// are all load-bearing rather than belt-and-braces:
    ///
    /// - The queue holds *only* over-budget refusals: entries are added
    ///   solely on the [`Self::refuse_bind`]-true path in the four capped
    ///   binds, so removing one removes a refusal and nothing else.
    /// - Removal is by exact [`ObjectId`] -- serial included -- so only the
    ///   refused object itself matches, never another bind reusing its
    ///   numeric protocol id.
    /// - The ordinary stop-then-`finished` flow never consults this queue:
    ///   its `finished` is sent inline in the request handler,
    ///   unconditionally. Withdrawing a queued refusal therefore cannot take
    ///   away an event the normal flow would have sent -- which the
    ///   per-protocol stop tests (exactly one `finished` each, all passing)
    ///   pin from the other side.
    pub(super) fn undefer_bind_refusal(&mut self, id: &ObjectId) {
        if self.refused.is_empty() {
            return;
        }
        self.refused.retain(|refused| refused.id() != *id);
    }

    /// Sends every deferred refusal's `finished`. Runs on loop idle (see
    /// [`State::defer_bind_refusal`]), never in `bind` -- by then the bind
    /// epilogue has assigned the objects their user data, so a destructor
    /// `finished` no longer pulls the object out from under it.
    fn finish_refused(&mut self) {
        for refused in self.refused.drain(..) {
            refused.finish();
        }
    }
}

/// A bind refused for being over budget, waiting for the event loop to go
/// idle so its `finished` can be sent.
///
/// The deferral is load-bearing, not tidiness. Wayland-backend's bind
/// epilogue unconditionally assigns user data to the just-bound object
/// (`rs/server_impl/common_poll.rs`'s `Bind` arm ends in
/// `client.map.with(object.id, ...).unwrap()`), and sending a destructor
/// event -- `finished` on three of the four capped globals -- destroys the
/// object first, so the epilogue's `unwrap` panics and takes every client's
/// session down with it. (Found the hard way: the first version of this
/// mechanism sent `finished` inline and died exactly there.) The refused bind
/// holds no budget claim and no handles in the meantime, so the wait changes
/// nothing but the timing: the callback runs once the loop goes idle --
/// normally in the same pass, later under a saturated one -- and the client
/// still learns of the refusal on its next round trip.
///
/// A client that stops the refused object before the callback runs still gets
/// exactly one `finished`: `stop` retracts the queued refusal first (see
/// [`BindBudget::undefer_bind_refusal`]) and then sends its own. Only the
/// ext list needs the retraction on the wire -- its `finished` is a plain
/// event, so without it the idle send would be a live duplicate. The other
/// three globals' `finished` events are destructors: stopping a refused bind
/// there destroys the object first, and the idle send lands on a dead object
/// and is swallowed as `InvalidId` (the generated `let _ =`) before any byte
/// is written. A refusal that outlives its object any other way -- the client
/// disconnects mid-batch, where no hook runs before cleanup -- likewise ends
/// swallowed, and the `destroyed` release finds nothing to remove.
#[derive(Debug)]
pub(super) enum RefusedBind {
    Workspace(ExtWorkspaceManagerV1),
    ToplevelList(ExtForeignToplevelListV1),
    WlrToplevel(ZwlrForeignToplevelManagerV1),
    /// The output manager's `done` carries a serial, captured at defer time:
    /// the refused manager will never build a configuration, so it is purely
    /// informational, but `done` without one is not a well-formed batch.
    OutputManager(ZwlrOutputManagerV1, u32),
}

impl RefusedBind {
    /// The refused object itself, for retracting the refusal if the client
    /// stops or destroys it before the idle send (see
    /// [`BindBudget::undefer_bind_refusal`]).
    fn id(&self) -> ObjectId {
        match self {
            Self::Workspace(manager) => manager.id(),
            Self::ToplevelList(list) => list.id(),
            Self::WlrToplevel(manager) => manager.id(),
            Self::OutputManager(manager, _) => manager.id(),
        }
    }

    /// Sends the refusal: each global's own teardown, never a protocol
    /// error. A miscount must not kill a legitimate shell -- and per-client
    /// by construction, since only the overflowing client's object is told.
    fn finish(self) {
        match self {
            Self::Workspace(manager) => {
                manager.done();
                manager.finished();
            }
            Self::ToplevelList(list) => list.finished(),
            Self::WlrToplevel(manager) => manager.finished(),
            Self::OutputManager(manager, serial) => {
                manager.done(serial);
                manager.finished();
            }
        }
    }
}

impl State {
    /// Defers a refused bind's `finished` to loop idle. See [`RefusedBind`]
    /// for why the send cannot happen in `bind`.
    ///
    /// Schedules the draining idle callback when the first refusal lands; it
    /// sends every refusal queued by then, so a batch of refusals costs one
    /// callback rather than one each.
    pub(super) fn defer_bind_refusal(&mut self, refused: RefusedBind) {
        let fresh = self.bind_budget.refused.is_empty();
        self.bind_budget.refused.push(refused);
        if fresh {
            self.loop_handle
                .insert_idle(|state| state.bind_budget.finish_refused());
        }
    }
}
