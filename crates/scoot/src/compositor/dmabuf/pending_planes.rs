//! How many plane fds one client may have this compositor hold in
//! `zwp_linux_buffer_params_v1` objects that have not become a buffer yet.
//!
//! Every `add` hands over a plane's fd, and the params object keeps it
//! (Smithay's `DmabufParamsData::planes`) until one of three things happens:
//! the params object is consumed by `create`/`create_immed`, which moves the
//! planes into a `Dmabuf`; it is destroyed; or its client disconnects. Nothing
//! else bounds that. The live-`wl_buffer` count (`wl_buffers.rs`) counts only
//! buffers that were created, and a params object is not a buffer. So a
//! client that creates params, adds planes and never creates anything could
//! make this process hold fds that no per-client count saw. Review of PR #233
//! measured 220 params x 4 adds holding 927 fds on a 1024-fd table. New
//! clients were shed at accept, and the fd-pressure guard could not pick the
//! offender, because none of its counted creations was past a grace (see
//! `docs/backlog/resolved/client-held-fd-bound-done.md`). This is on every
//! tier that offers the dmabuf global, the default pixman one included.
//!
//! Every plane's fd is also recorded in the per-client fd ledger
//! (`client_fds.rs`) on `add`, and stays counted there, as the client's, for
//! as long as the fd is open: pending here, then in the `Dmabuf`, then for as
//! long as a surface keeps the buffer committed after its `wl_buffer` is
//! destroyed. That ledger is what bounds a client's plane fds overall (512
//! fds of every kind) and what fd pressure reads. This count bounds the
//! narrower shape the ledger alone would let reach 512: planes parked in
//! params objects that never become buffers at all.
//!
//! ## The number
//!
//! [`MAX_PENDING_PLANES_PER_CLIENT`] is 32. The protocol's usage pattern is
//! to send `add` for each plane and then `create`/`create_immed` for the
//! same params object straight away: nothing a client learns in between
//! changes what it would send, and `create`'s answer arrives only after the
//! planes are consumed. So a client building buffers one at a time has at
//! most one buffer's planes pending. A buffer has at most 4 planes
//! (`MAX_PLANES`), so that is 4 fds. That is reasoned from the protocol, not
//! verified client by client (this run could not reach Mesa's source, and
//! nothing on the dev VM makes a dma-buf; see below). On `create`,
//! Smithay drains the planes into the `Dmabuf` at request time, whatever the
//! import's outcome, so an async `create` still waiting for `created` holds
//! none on the params. 32 is eight whole four-plane buffers mid-construction,
//! 8x that maximum, which leaves room for a client that interleaves the
//! construction of several buffers. It is generous on purpose, because
//! tripping it disconnects the client.
//!
//! The dev VM cannot show this on the wire: no real client there makes a
//! dma-buf, since `gbm_bo_create` on its render node is refused and Mesa
//! falls back to `wl_shm` (see `dmabuf.rs`). A wire capture of real GPU
//! clients belongs with the real-hardware checks (`Asahi.md`).
//!
//! There is no pressure grace of its own any more (it was 8). Under fd
//! pressure an `add` is refused on the client's whole fd count in the ledger
//! (past `client_fds::PRESSURE_GRACE_FDS`, 128), which sees these planes and
//! every other fd the client made scoot keep; a count of params planes alone
//! could only pick the smaller part of a client's footprint.
//!
//! ## How it is counted
//!
//! Per params object, keyed by its [`ObjectId`], serial included, so a
//! numeric id reused by a later object never collides. There is also a
//! per-client sum, which is what the bound reads. The count follows the fds
//! exactly on every path, with no phantom left behind:
//!
//! - **Claimed on `add`**, before delegation. That includes an `add` Smithay
//!   then refuses (`already_used`, `plane_idx`, `plane_set`), whose fd it
//!   drops as it kills the client. That claim is released by the destroy
//!   below when the dead client's objects are cleaned up. It is keyed by the
//!   params object, not left on a client entry, so unlike `wl_buffers.rs`'s
//!   phantom unit it does not outlive the connection.
//! - **Released on `create`/`create_immed`**, the consuming requests, before
//!   delegation. From there the planes belong to a `Dmabuf`, which is either
//!   a `wl_buffer` (its fds still counted in the fd ledger, where they have
//!   been since their `add`) or dropped at once when the import is refused. A consume that `dispatch.rs`'s buffer cap refuses
//!   instead kills the client, and the destroy below releases it.
//! - **Released when the params object is destroyed**, in `dispatch.rs`'s
//!   blanket `destroyed` hook. That covers an explicit `destroy`, a
//!   disconnect and a protocol-error kill. The release removes the object's
//!   entry, so a consume followed by a destroy releases once.
//!
//! The release lands when the object is destroyed. The fds themselves close a
//! moment later in the same dispatch, when wayland-backend drops the object
//! data. No request of the same client runs in between.
//!
//! ## Refusal form
//!
//! A refused `add`, by this bound or by the fd ledger's, disconnects the
//! client with `wl_display.error(no_memory)` (see `no_memory.rs`). The params interface's own errors
//! (`plane_idx`, `plane_set`, `incomplete`, ...) all describe a malformed
//! plane, and this is not one. The request is not delegated, so its fd is
//! dropped with it.
//!
//! ## Cost
//!
//! `add` is on the path of every dma-buf a GPU client allocates (once per
//! buffer it allocates, not per frame). On that path this is a `TypeId`
//! comparison, a downcast, and one lookup and one insert in each of two maps
//! whose capacity persists, plus the fd ledger's arrival: a lookup to forget
//! any record on the same number, two map lookups for the bounds, one
//! `fstat` for the plane's identity, and one insert in each of the ledger's
//! two maps. The fd-pressure observation runs only past the grace. Measured
//! in the PR that added the ledger (see
//! `docs/backlog/resolved/buffer-fds-past-their-object-done.md`).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd};

use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::{Client, Resource};

use crate::compositor::client_fds::Kind;
use crate::compositor::{State, no_memory};

#[cfg(test)]
mod tests;

/// How many plane fds one client may have held in params objects that have
/// not been consumed yet, across all of them.
///
/// 32: eight whole four-plane buffers mid-construction, 8x the one buffer's
/// worth (at most 4) a client building buffers one at a time has in flight.
/// See the module doc. Past it the `add` is refused by disconnecting the client with
/// `wl_display.error(no_memory)`.
pub(in crate::compositor) const MAX_PENDING_PLANES_PER_CLIENT: u32 = 32;

/// The per-params and per-client plane counts. See the module doc.
#[derive(Debug, Default)]
pub struct PendingPlanes {
    /// Planes added to each live params object that has not been consumed.
    /// An entry exists only while it holds at least one. It is removed on
    /// consume and on destroy, so the map is bounded by live params objects
    /// that hold planes, and by the per-client cap.
    per_params: HashMap<ObjectId, u32>,
    /// The sum of `per_params` over each client's params objects. An entry
    /// exists only while it is nonzero.
    per_client: HashMap<ClientId, u32>,
}

impl PendingPlanes {
    /// How many pending plane fds `client` has this compositor hold.
    /// Test-only: the guard reads the count through [`Self::try_claim`].
    #[cfg(test)]
    pub(in crate::compositor) fn live_for(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).copied().unwrap_or(0)
    }

    /// Counts one more plane on `params` for `client`, unless it already
    /// holds [`MAX_PENDING_PLANES_PER_CLIENT`], in which case nothing is
    /// counted and how many it holds is handed back.
    ///
    /// One lookup in each map on the admitted path, which is every `add` of
    /// every well-behaved client. The counts cannot overflow: each unit is a
    /// live fd in this process's table, and the cap stops a client far below
    /// `u32::MAX`.
    fn try_claim(&mut self, client: &ClientId, params: ObjectId) -> Result<(), u32> {
        let live = self.per_client.entry(client.clone()).or_insert(0);
        if *live >= MAX_PENDING_PLANES_PER_CLIENT {
            return Err(*live);
        }
        *live += 1;
        *self.per_params.entry(params).or_insert(0) += 1;
        Ok(())
    }

    /// Forgets every plane `params` holds: it was consumed, or destroyed.
    /// Idempotent, since the entry is removed. A params object that holds
    /// none, or that was already released, is one failed lookup.
    pub(in crate::compositor) fn release(&mut self, client: &ClientId, params: &ObjectId) {
        let Some(planes) = self.per_params.remove(params) else {
            return;
        };
        if let Some(live) = self.per_client.get_mut(client) {
            *live = live.saturating_sub(planes);
            if *live == 0 {
                self.per_client.remove(client);
            }
        }
    }

    /// How many planes every client holds between them. Test-only.
    #[cfg(test)]
    pub(in crate::compositor) fn in_flight(&self) -> u32 {
        self.per_client.values().sum()
    }

    /// How many params objects hold at least one plane. Test-only: pins
    /// that the per-object map drains along with the sums.
    #[cfg(test)]
    pub(in crate::compositor) fn params_tracked(&self) -> usize {
        self.per_params.len()
    }
}

/// Disconnects the client and returns `true` when `request` is a
/// `zwp_linux_buffer_params_v1.add` past the client's pending-plane cap, or
/// past its fd bound in the fd ledger (`client_fds.rs`: 512 fds of every
/// kind, or the 128-fd grace while the fd table is pressured). Otherwise it
/// counts the plane here, records its fd in the ledger, and returns `false`.
///
/// Both bounds are checked before either claims, so a refusal never takes a
/// unit or a record it would then have to give back. The pressure
/// observation (`getrlimit` and a `/proc/self/fd` readdir) runs only for a
/// client already past the grace, at most once per `client_fds::SWEEP_MARGIN`
/// arrivals.
///
/// Folds away for every interface other than the params one, for the same
/// monomorphization reason as `dispatch.rs`'s guards: this runs on every
/// request of every interface.
pub(in crate::compositor) fn reject_excess_plane<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<zwp_linux_buffer_params_v1::Request>() {
        return false;
    }
    let Some(zwp_linux_buffer_params_v1::Request::Add { fd, .. }) =
        (request as &dyn Any).downcast_ref::<zwp_linux_buffer_params_v1::Request>()
    else {
        return false;
    };
    const REFUSED: &str = "zwp_linux_buffer_params_v1.add refused";
    let id = client.id();
    // Forgets whatever the ledger recorded on this number first: it arrived
    // again, so that fd was closed.
    let message = match state
        .client_fds
        .admit_arrival(&id, fd.as_raw_fd(), Kind::Plane)
    {
        Err(refusal) => refusal.message(REFUSED),
        Ok(()) => match state.pending_planes.try_claim(&id, resource.id()) {
            Ok(()) => {
                state
                    .client_fds
                    .record_arrival(&id, fd.as_fd(), Kind::Plane);
                return false;
            }
            Err(live) => format!(
                "{REFUSED}: this client already has {live} dmabuf planes added to params \
                 objects it has not created a buffer from (the bound is \
                 {MAX_PENDING_PLANES_PER_CLIENT})"
            ),
        },
    };
    tracing::debug!(%message, "refusing a dmabuf plane (no_memory)");
    no_memory::disconnect(&state.display_handle, client, message);
    true
}

/// Releases a params object's planes when `request` consumes it
/// (`create`/`create_immed`). Smithay moves the planes into a `Dmabuf` right
/// there, so from this point they are a buffer's, or they are dropped.
///
/// Runs after the guard chain, for a request that is about to be delegated.
/// A consume the buffer cap refused instead has killed its client, and the
/// params destroy that disconnect runs releases it; the entry is still there
/// for it to find.
pub(in crate::compositor) fn note_consumed<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<zwp_linux_buffer_params_v1::Request>() {
        return;
    }
    let consumes = matches!(
        (request as &dyn Any).downcast_ref::<zwp_linux_buffer_params_v1::Request>(),
        Some(
            zwp_linux_buffer_params_v1::Request::Create { .. }
                | zwp_linux_buffer_params_v1::Request::CreateImmed { .. }
        )
    );
    if consumes {
        state.pending_planes.release(&client.id(), &resource.id());
    }
}

/// Releases a destroyed params object's planes: an explicit destroy, a
/// disconnect or a kill. Called from `dispatch.rs`'s blanket `destroyed`.
/// Folds away for every other interface.
pub(in crate::compositor) fn forget_destroyed<I>(state: &mut State, client: &ClientId, resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<zwp_linux_buffer_params_v1::Request>() {
        return;
    }
    state.pending_planes.release(client, &resource.id());
}
