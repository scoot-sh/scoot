//! Explicit sync: `wp_linux_drm_syncobj_manager_v1` (`linux-drm-syncobj-v1`).
//!
//! A GPU client using explicit sync attaches a dma-buf together with two
//! points on DRM timeline syncobjs: an **acquire** point the client's GPU
//! signals when the buffer is finished, and a **release** point the
//! compositor signals when it is done reading the buffer. NVIDIA's driver
//! effectively requires this, and Mesa's Vulkan WSI prefers it; without it a
//! GPU client leans on implicit fencing, which not every driver provides.
//!
//! Smithay at the pinned rev carries the protocol (`wayland/drm_syncobj/`):
//! the global, the timeline import, the per-surface objects, every
//! protocol error, and signalling a release point when the last reference
//! to the buffer it rode in on is dropped (`renderer/utils/wayland.rs`'s
//! `InnerBuffer::drop`). What it leaves to the compositor, and this module
//! plus `tty/scanout.rs` supply, is:
//!
//! - **Where the global exists** ([`DrmSyncobj::enable`]). Only on the
//!   `--tty` GPU scanout tier, and only when a DRM device passes Smithay's
//!   own probe (`supports_syncobj_eventfd`: the kernel answers
//!   `DRM_IOCTL_SYNCOBJ_EVENTFD` with `ENOENT` for a missing handle rather
//!   than refusing the ioctl). Never on pixman (dumb `--tty`, headless,
//!   nested): nothing there could wait on an acquire point without blocking
//!   the event loop, and a client that binds the global and has its points
//!   ignored is worse off than one that never saw it. The import device --
//!   the one timelines are imported into and waited on -- is the session's
//!   own DRM fd (`DrmDevice::device_fd`) if it passes the probe, else the
//!   first render node (`/dev/dri/renderD*`, in name order) that does (a
//!   split render/display machine; see `enable`). Syncobj ioctls are `DRM_RENDER_ALLOW` and need no master, so
//!   they keep working while the session is VT-switched away.
//! - **Waiting on acquire points** before a commit applies
//!   ([`acquire`]): the blocker pattern anvil shows, plus what anvil leaves
//!   out -- an already-signalled point costs one ioctl and no eventfd, a
//!   surface destroyed mid-wait (including by a disconnect) takes its
//!   eventfd sources with it, and outstanding waits are bounded per client.
//!   Without the wait, direct scanout would flip an unfinished buffer:
//!   `DrmCompositor` hands KMS an already-signalled fence for any explicit
//!   buffer (`ScanoutBuffer::acquire_point` at the pinned rev), on the
//!   assumption that a blocker already waited.
//! - **Releasing only when the GPU is done** on the composited path
//!   ([`ExplicitBuffers`] here, the hold in `tty/scanout.rs`): Smithay
//!   signals a release point from the CPU the moment a surface replaces its
//!   buffer, while a queued frame may still be sampling it on the GPU.
//! - **Bounds** on what a client can make the compositor hold: imported
//!   timeline fds, live objects and destroyed ones that sync points still
//!   reference ([`MAX_TIMELINES_PER_CLIENT`], kept by [`retained`]), and
//!   outstanding acquire waits ([`MAX_ACQUIRE_WAITS_PER_CLIENT`], each an
//!   eventfd plus a queued transaction Smithay scans on every commit of that
//!   client).
//!
//! ## What is still Smithay's, and one thing upstream gets wrong
//!
//! Every protocol error (`no_buffer`, `no_acquire_point`,
//! `no_release_point`, `conflicting_points`, `unsupported_buffer`,
//! `surface_exists`, `invalid_timeline`, `no_surface`) is posted by
//! Smithay's own handlers and pre-commit hook; scoot adds none of those and
//! forwards every request to them through `dispatch.rs`'s blanket impl.
//!
//! **Upstream Smithay never destroys an imported timeline's syncobj
//! handle**, which is why scoot builds against a scoot-sh fork (see
//! `crates/scoot/Cargo.toml`). Upstream's `DrmTimelineInner` has no `Drop`,
//! and nothing but `invalidate()` calls `destroy_syncobj`, at `0ff0098` and
//! on `master` alike. So every `import_timeline` would leave one handle on
//! the import device's DRM file until scoot exits, keeping the client's
//! syncobj -- and any acquire wait scoot abandoned on it -- alive with it:
//! about 80 bytes of kernel slab per import and 200 per abandoned wait,
//! drivable at wire speed. There is no safe scoot-side reclaim: rotating
//! the import device (`close_device` + `update_device`) would silently stop
//! the release points of timelines a client destroyed while points on them
//! are still in flight, because Smithay forgets those timelines on destroy
//! while the points keep only a weak reference to the device. The fork adds
//! the one-line `Drop` (`DrmTimelineDeviceSpecific`), and with it both
//! leaks measure as noise. See
//! `docs/backlog/resolved/syncobj-handle-leak-done.md`.

pub(crate) mod acquire;
#[cfg(any(feature = "gpu-scanout", test))]
pub(crate) mod release_hold;
pub(crate) mod retained;

#[cfg(test)]
mod tests;

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::collections::HashSet;
use std::os::fd::{AsRawFd, RawFd};

use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::wayland::drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd};

use super::State;

/// How many imported syncobj timelines one client may have this process
/// hold at once, live objects and destroyed-but-still-referenced ones alike.
///
/// Sized against real use: Mesa's Vulkan WSI imports two timelines per
/// swapchain image (acquire and release, `wsi_common_wayland.c`), so a
/// four-image swapchain is 8, and an old swapchain still alive while its
/// replacement is built doubles that -- 16 per window. 128 is eight such
/// windows at once.
///
/// **What is counted is the fd, not the object.** Each import keeps the
/// client's syncobj fd open here until the last reference to the timeline
/// goes. Every sync point on it is such a reference, and a point outlives the
/// timeline object's destruction (the protocol says so). So the count is
/// released when the fd really closes, which [`retained`] observes, and not
/// when the object is destroyed. An earlier version counted objects and
/// released on destroy. Review measured 440 surfaces with pending points on
/// destroyed timelines holding 927 fds with nothing counted
/// (`docs/backlog/resolved/client-held-fd-bound-done.md`).
///
/// An import that finds the client at this bound sweeps its records first,
/// and is refused only if fewer than [`retained::SWEEP_MARGIN`] of them turn
/// out to be closed: a client really holding more than 112. The refusal is
/// the protocol's own `invalid_timeline` on the manager, killing only that
/// client. So a client never has more than 128 timeline fds held here.
pub(crate) const MAX_TIMELINES_PER_CLIENT: u32 = 128;

/// How many commits one client may have waiting on unsignalled acquire
/// points at once, across all its surfaces.
///
/// Each costs an eventfd and a calloop source here, and a blocked
/// transaction in the client's queue -- which Smithay scans linearly, with a
/// `Vec::remove`, on every commit that client makes (`TransactionQueue::
/// take_ready`), so an unbounded queue would be fd exhaustion *and*
/// quadratic work on the event loop. A legitimate client has at most one
/// wait per frame its GPU is behind on, per surface -- a Vulkan swapchain
/// cannot run more than its image count ahead -- so 64 is several busy
/// surfaces each several frames behind. Past it the client is disconnected
/// with `wl_display.no_memory` (see [`acquire`] for why that code).
pub(crate) const MAX_ACQUIRE_WAITS_PER_CLIENT: u32 = 64;

/// Timelines a client may hold before compositor-wide fd pressure starts
/// refusing its imports. Same conditional shape as
/// `fd_pressure::PRESSURE_GRACE_BUFFERS`: only ever enforced while the fd
/// table is pressured, so a client under it is never refused for another
/// client's greed. 32 is two windows' worth (see
/// [`MAX_TIMELINES_PER_CLIENT`]). Held means retained, as for the cap, and a
/// refusal is decided on a fresh sweep. Sweeps under pressure are amortized
/// by admitting up to [`retained::SWEEP_MARGIN`] imports past a sweep that
/// left the client at or under the grace, so under pressure a client holds
/// at most 32 + 16.
pub(crate) const PRESSURE_GRACE_TIMELINES: u32 = 32;

/// Outstanding acquire waits a client may hold before fd pressure starts
/// disconnecting it on the next wait. 16 is a few surfaces a few frames
/// behind.
pub(crate) const PRESSURE_GRACE_ACQUIRE_WAITS: u32 = 16;

/// Everything scoot keeps for explicit sync. See the module doc.
#[derive(Debug, Default)]
pub struct DrmSyncobj {
    /// Smithay's protocol state, and with it the global. `None` wherever the
    /// global is not offered -- every session but the GPU scanout tier on a
    /// device that passed the probe -- which is also what
    /// [`DrmSyncobjHandler::drm_syncobj_state`] answers.
    state: Option<DrmSyncobjState>,
    /// The imported timeline fds this process still holds, per client: what
    /// [`MAX_TIMELINES_PER_CLIENT`] bounds. Recorded in `dispatch.rs` before
    /// an import is delegated, and forgotten when the fd is seen closed (see
    /// [`retained`]), not when the timeline object is destroyed.
    timelines: retained::RetainedTimelines,
    /// The acquire side's bookkeeping: outstanding waits per surface and per
    /// client. See [`acquire`].
    pub(super) waits: acquire::Waits,
    /// Which `wl_buffer`s were last committed with sync points.
    explicit: ExplicitBuffers,
}

impl DrmSyncobj {
    /// Offers the global on the first of `candidates` that passes Smithay's
    /// syncobj-eventfd probe, and answers whether one did.
    ///
    /// Called once, by `tty::init`, when the session came up on the GPU
    /// scanout tier -- the only tier where an acquire point can be waited on
    /// without blocking and a composited frame's GPU work can be waited out
    /// before a release point is signalled. `candidates` is lazy: each is a
    /// name for the log and the device, or `None` if it could not be opened,
    /// and nothing after the first that passes is opened at all. Idempotent:
    /// a second call keeps the first global.
    ///
    /// Why more than one candidate: a syncobj is a DRM-core object, not a
    /// driver's, so any DRM device whose driver supports timeline syncobjs
    /// can import a client's timeline and wait on it, whichever GPU created
    /// it. The session's own display device is tried first (it is already
    /// open, and on a single-GPU machine it is the GPU). On a split
    /// render/display machine -- Apple Silicon's `apple,dcp` display
    /// controller beside the AGX GPU, most ARM SoCs -- the display driver
    /// may have no syncobj support while the render node clients render on
    /// does, and without further candidates those machines would never be
    /// offered explicit sync. `tty::init` passes every `/dev/dri/renderD*`
    /// after the display device, in name order.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
    pub(crate) fn enable<I>(&mut self, display: &DisplayHandle, candidates: I) -> bool
    where
        I: IntoIterator<Item = (Cow<'static, str>, Option<DrmDeviceFd>)>,
    {
        if self.state.is_some() {
            return true;
        }
        for (name, device) in candidates {
            let name: &str = &name;
            let Some(device) = device else {
                tracing::debug!(
                    device = name,
                    "drm: explicit-sync candidate could not be opened"
                );
                continue;
            };
            if !supports_syncobj_eventfd(&device) {
                tracing::debug!(
                    device = name,
                    "drm: explicit-sync candidate has no syncobj timeline eventfd support"
                );
                continue;
            }
            self.state = Some(DrmSyncobjState::new::<State>(display, device));
            // info!, like the tier line: whether GPU clients get explicit sync
            // on this machine is a question a user asks of the log.
            tracing::info!(
                device = name,
                "drm: explicit sync (wp_linux_drm_syncobj_manager_v1) offered"
            );
            return true;
        }
        tracing::info!(
            "drm: no device here has syncobj timeline eventfd support; explicit sync \
             (wp_linux_drm_syncobj_manager_v1) is not offered"
        );
        false
    }

    /// Whether the global exists, and so whether any surface can carry sync
    /// points at all. Every per-commit and per-frame path checks this first.
    pub(crate) fn active(&self) -> bool {
        self.state.is_some()
    }

    /// The explicit-buffer classification the scanout tier's release hold
    /// reads every frame.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
    pub(crate) fn explicit(&self) -> &ExplicitBuffers {
        &self.explicit
    }

    /// `fd` has just been received from a client in some request, so any
    /// timeline recorded on that number was closed. See
    /// [`retained::RetainedTimelines::fd_arrived`].
    pub(crate) fn fd_arrived(&mut self, fd: RawFd) {
        self.timelines.fd_arrived(fd);
    }

    /// Sweeps every client's timeline records and answers how many
    /// timeline fds this process still holds between them. Test-only.
    #[cfg(test)]
    pub(crate) fn timelines_in_flight(&mut self) -> u32 {
        self.timelines.sweep_all()
    }
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.drm_syncobj.state.as_mut()
    }
}

/// The `wl_buffer`s whose most recent commit carried sync points.
///
/// The release hold in `tty/scanout.rs` keeps exactly these alive until the
/// composited frame that sampled them is done on the GPU; every other buffer
/// keeps today's release timing, so implicit-sync clients see no change at
/// all. Keyed by the buffer object, because that is all a render element
/// exposes: `WaylandSurfaceRenderElement::buffer` gives the `Buffer` but not
/// its surface, and `Buffer::acquire_point` is `pub(crate)` in Smithay.
///
/// Written by the acquire hook on every commit that attaches a buffer
/// (explicit inserts, implicit removes), pruned when the buffer object is
/// destroyed (`dispatch.rs`), so it is bounded by live `wl_buffer`s. One
/// known imprecision, inherited from Smithay rather than introduced here:
/// re-committing the *same* `wl_buffer` keeps the `Buffer` (and the sync
/// points) of its first commit (`RendererSurfaceState::update_buffer` only
/// replaces a buffer that differs), so a buffer re-committed implicitly
/// after an explicit commit is classified implicit while its `Buffer` still
/// carries the old release point. Only a client switching sync modes on one
/// buffer without an intervening attach reaches it, and the cost is the
/// release timing this set exists to fix, for that one buffer.
#[derive(Debug, Default)]
pub struct ExplicitBuffers {
    ids: HashSet<ObjectId>,
}

impl ExplicitBuffers {
    /// Whether no buffer is explicit -- the common case, and the per-frame
    /// fast path: an empty set is one length check.
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Whether `buffer`'s most recent commit carried sync points.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
    pub(crate) fn contains(&self, buffer: &WlBuffer) -> bool {
        !self.ids.is_empty() && self.ids.contains(&buffer.id())
    }

    /// Records a commit of `buffer` with (`explicit`) or without sync points.
    /// An implicit commit on an empty set touches nothing.
    pub(crate) fn note(&mut self, buffer: &WlBuffer, explicit: bool) {
        if explicit {
            self.ids.insert(buffer.id());
        } else if !self.ids.is_empty() {
            self.ids.remove(&buffer.id());
        }
    }

    /// Forgets a destroyed buffer object.
    fn forget(&mut self, buffer: &ObjectId) {
        self.ids.remove(buffer);
    }

    /// How many buffers are classified explicit. Test-only.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Refuses a `wp_linux_drm_syncobj_manager_v1.import_timeline` past the
/// per-client retained-timeline cap, or past its pressure grace while the fd
/// table is pressured, with the protocol's own `invalid_timeline` on the
/// manager. Otherwise it records the import's fd against the client and
/// answers `false`, so the import is delegated.
///
/// The decision is [`retained::RetainedTimelines::admit`]'s. A refusal is
/// only ever decided on a fresh sweep, because the records include dead ones
/// until something notices. See [`MAX_TIMELINES_PER_CLIENT`] and
/// [`PRESSURE_GRACE_TIMELINES`] for the two rules.
///
/// Recorded before delegation, from the request's own fd number, which
/// Smithay moves unchanged into the timeline (`DrmTimeline::new`). An import
/// Smithay then refuses (the device cannot import the fd) drops the fd and
/// kills the client. That leaves a record on a closed number for a dead
/// client, bounded like every other one (see [`retained`]).
///
/// Posting the error kills the client synchronously, which is what makes
/// returning without initialising the request's `New` safe -- the argument
/// `dispatch.rs`'s module doc makes for every guard there.
///
/// Folds away for every other interface (`TypeId` comparison on a
/// monomorphised type), so the blanket dispatch pays nothing for it.
pub(super) fn reject_excess_timeline<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wp_linux_drm_syncobj_manager_v1::Request>() {
        return false;
    }
    let Some(wp_linux_drm_syncobj_manager_v1::Request::ImportTimeline { fd, .. }) =
        (request as &dyn Any).downcast_ref::<wp_linux_drm_syncobj_manager_v1::Request>()
    else {
        return false;
    };
    let fd = fd.as_raw_fd();
    let id = client.id();
    let timelines = &mut state.drm_syncobj.timelines;
    // Whatever was recorded on this number was closed, or the kernel could
    // not have handed it out again. Forgotten before the bound is read, so
    // it is not counted against anyone.
    timelines.fd_arrived(fd);
    let verdict = timelines.admit(
        &id,
        MAX_TIMELINES_PER_CLIENT,
        PRESSURE_GRACE_TIMELINES,
        retained::timeline_fd_open,
        || super::fd_pressure::table().is_some_and(|table| table.pressured()),
    );
    let message = match verdict {
        Ok(()) => {
            timelines.record(&id, fd);
            return false;
        }
        Err(retained::Refusal::Cap { held }) => format!(
            "timeline refused: this compositor still holds {held} of this client's imported \
             timelines (live ones, and destroyed ones its sync points still reference), and \
             the maximum is {MAX_TIMELINES_PER_CLIENT}"
        ),
        Err(retained::Refusal::Pressure { held }) => format!(
            "timeline refused: compositor-wide file-descriptor pressure, and this compositor \
             still holds {held} of this client's imported timelines, more than the \
             {PRESSURE_GRACE_TIMELINES}-timeline pressure grace"
        ),
    };
    tracing::debug!(%message, "refusing a syncobj timeline import (protocol error)");
    resource.post_error(
        wp_linux_drm_syncobj_manager_v1::Error::InvalidTimeline,
        message,
    );
    true
}

/// The explicit-buffer set's pruning: called from `dispatch.rs`'s
/// `destroyed` for every object, folding away for all but `wl_buffer`.
///
/// Destroying a timeline object deliberately touches nothing here any more.
/// Its fd stays open while any sync point on it survives, and the ledger
/// notices when it really closes (see [`retained`]).
pub(super) fn forget_destroyed<I>(state: &mut State, resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    use smithay::reexports::wayland_server::protocol::wl_buffer;
    if TypeId::of::<I::Request>() == TypeId::of::<wl_buffer::Request>()
        && !state.drm_syncobj.explicit.is_empty()
    {
        state.drm_syncobj.explicit.forget(&resource.id());
    }
}
