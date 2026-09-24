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
//! ## The number
//!
//! [`MAX_PENDING_PLANES_PER_CLIENT`] is 32. No client this project knows of
//! has more than one buffer's planes in flight at once. Mesa's EGL and Vulkan
//! WSI, GStreamer's waylandsink, mpv, Firefox and Chromium all send `add` for
//! each plane and then `create`/`create_immed` for the same params object
//! back to back, in one flush. A buffer has at most 4 planes
//! (`MAX_PLANES`), so that is at most 4 pending plane fds. On `create`,
//! Smithay drains the planes into the `Dmabuf` at request time, whatever the
//! import's outcome, so an async `create` still waiting for `created` holds
//! none on the params. 32 is eight whole four-plane buffers mid-construction,
//! 8x that maximum. It is generous on purpose, because tripping it disconnects
//! the client.
//!
//! The dev VM cannot show this on the wire: no real client there makes a
//! dma-buf, since `gbm_bo_create` on its render node is refused and Mesa
//! falls back to `wl_shm` (see `dmabuf.rs`). The maximum above therefore comes
//! from those clients' source, not from a capture.
//!
//! [`PRESSURE_GRACE_PENDING_PLANES`] is 8, two whole four-plane buffers. It
//! applies only while the fd table is pressured (see `fd_pressure.rs`),
//! so a client under it is never refused for another client's greed.
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
//!   a `wl_buffer` (counted by `wl_buffers.rs`) or dropped at once when the
//!   import is refused. A consume that `dispatch.rs`'s buffer cap refuses
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
//! A refused `add` disconnects the client with `wl_display.error(no_memory)`
//! (see `no_memory.rs`). The params interface's own errors
//! (`plane_idx`, `plane_set`, `incomplete`, ...) all describe a malformed
//! plane, and this is not one. The request is not delegated, so its fd is
//! dropped with it.
//!
//! ## Cost
//!
//! `add` is on the path of every dma-buf a GPU client allocates. On that
//! path this is a `TypeId` comparison, a downcast, and one lookup and one
//! insert in each of two maps whose capacity persists. There are no
//! syscalls: the fd-pressure observation runs only past the grace. It also
//! does one lookup to invalidate a stale syncobj timeline record on the same
//! fd number (see `drm_syncobj/retained.rs`), which is a single `is_empty`
//! test on every tier but the GPU scanout one.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::os::fd::AsRawFd;

use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::{Client, Resource};

use crate::compositor::{State, no_memory};

#[cfg(test)]
mod tests;

/// How many plane fds one client may have held in params objects that have
/// not been consumed yet, across all of them.
///
/// 32: eight whole four-plane buffers mid-construction, 8x the one buffer's
/// worth (at most 4) that any known client ever has in flight. See the module
/// doc. Past it the `add` is refused by disconnecting the client with
/// `wl_display.error(no_memory)`.
pub(in crate::compositor) const MAX_PENDING_PLANES_PER_CLIENT: u32 = 32;

/// Pending planes a client may hold before fd pressure starts refusing its
/// `add`s. Same conditional shape as `fd_pressure::PRESSURE_GRACE_BUFFERS`.
/// 8 is two whole four-plane buffers, 2x any known client's maximum.
pub(in crate::compositor) const PRESSURE_GRACE_PENDING_PLANES: u32 = 8;

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

    /// Counts one more plane on `params` for `client`, unless `refuse`
    /// (given how many the client holds) says no, in which case nothing is
    /// counted and the refusal and that count are handed back.
    ///
    /// One lookup in each map on the admitted path, which is every `add` of
    /// every well-behaved client. The counts cannot overflow: each unit is a
    /// live fd in this process's table, and the cap stops a client far below
    /// `u32::MAX`.
    fn try_claim(
        &mut self,
        client: &ClientId,
        params: ObjectId,
        refuse: impl FnOnce(u32) -> Option<Refusal>,
    ) -> Result<(), (Refusal, u32)> {
        let live = self.per_client.entry(client.clone()).or_insert(0);
        if let Some(refusal) = refuse(*live) {
            let held = *live;
            if held == 0 {
                // Only a refusal at zero could have just made the entry,
                // which nothing does, but the invariant (no zero entries)
                // should not rest on that.
                self.per_client.remove(client);
            }
            return Err((refusal, held));
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
/// past its pressure grace while the fd table is pressured. Otherwise it
/// counts the plane and returns `false`.
///
/// Checked before the claim, so a refusal never takes a unit it would then
/// have to give back. The pressure observation (`getrlimit` and a
/// `/proc/self/fd` readdir) runs only for a client already past the grace;
/// see `dispatch::pressure_refusal`.
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
    // Whatever happens to this plane, its fd number is now this process's
    // again. That proves any syncobj timeline recorded on the same number
    // was closed; see `drm_syncobj/retained.rs`.
    state.drm_syncobj.fd_arrived(fd.as_raw_fd());
    let claimed = state
        .pending_planes
        .try_claim(&client.id(), resource.id(), |live| {
            plane_refusal(live, || {
                crate::compositor::fd_pressure::table().is_some_and(|table| table.pressured())
            })
        });
    let Err((refusal, live)) = claimed else {
        return false;
    };
    let (bound, why) = match refusal {
        Refusal::Cap => (MAX_PENDING_PLANES_PER_CLIENT, ""),
        Refusal::Pressure => (
            PRESSURE_GRACE_PENDING_PLANES,
            ", while compositor-wide file descriptors are under pressure",
        ),
    };
    tracing::debug!(
        live,
        bound,
        "refusing a dmabuf plane past the per-client pending-plane bound (no_memory)"
    );
    no_memory::disconnect(
        &state.display_handle,
        client,
        format!(
            "zwp_linux_buffer_params_v1.add refused: this client already has {live} dmabuf \
             planes added to params objects it has not created a buffer from (the bound is \
             {bound}{why})"
        ),
    );
    true
}

/// Why an `add` is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The client holds [`MAX_PENDING_PLANES_PER_CLIENT`] already.
    Cap,
    /// It holds more than [`PRESSURE_GRACE_PENDING_PLANES`], and the fd
    /// table is pressured.
    Pressure,
}

/// The decision inside [`reject_excess_plane`], for a client that holds
/// `live` pending planes. `pressured` observes the fd table, and is called
/// only past the grace, which is `dispatch::pressure_refusal`'s rule
/// exactly (`live > grace && pressured`). Split out so that both boundaries
/// are pinned without filling the test process's fd table.
fn plane_refusal(live: u32, pressured: impl FnOnce() -> bool) -> Option<Refusal> {
    if live >= MAX_PENDING_PLANES_PER_CLIENT {
        Some(Refusal::Cap)
    } else if live > PRESSURE_GRACE_PENDING_PLANES && pressured() {
        Some(Refusal::Pressure)
    } else {
        None
    }
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
