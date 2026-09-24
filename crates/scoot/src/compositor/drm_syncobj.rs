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
//! - **Bounds** on what a client can make the compositor hold: live
//!   timeline objects ([`MAX_TIMELINES_PER_CLIENT`]) and outstanding acquire
//!   waits ([`MAX_ACQUIRE_WAITS_PER_CLIENT`], each an eventfd plus a queued
//!   transaction Smithay scans on every commit of that client). The first
//!   is an object count, **not** an fd bound -- see its doc.
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

#[cfg(test)]
mod tests;

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::wayland::drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd};

use super::State;

/// How many live `wp_linux_drm_syncobj_timeline_v1` objects one client may
/// hold at once.
///
/// Sized against real use: Mesa's Vulkan WSI imports two timelines per
/// swapchain image (acquire and release, `wsi_common_wayland.c`), so a
/// four-image swapchain is 8, and an old swapchain still alive while its
/// replacement is built doubles that -- 16 per window. 128 is eight such
/// windows at once. Past it, the import is refused with the protocol's own
/// `invalid_timeline` on the manager, killing only that client.
///
/// **This bounds live timeline *objects*, not the fds they hold.** Each
/// imported timeline keeps the client's syncobj fd open in this process
/// (Smithay's `DrmTimelineInner` owns it), but so does every `DrmSyncPoint`
/// on it, which holds the timeline's `Arc` -- and a point set on a surface
/// outlives the timeline object's destruction (the protocol says destroying
/// a timeline does not unset its points). The count is released on destroy
/// regardless, so a client that imports, sets a point on a fresh surface
/// and destroys the timeline keeps one fd here per surface with none
/// counted: measured in review, 440 such surfaces held 927 fds with zero
/// live timelines, new clients were shed and the offender was not killed.
/// The same hole exists without syncobj, through `zwp_linux_buffer_params_v1`
/// adds that are never created. Both are
/// `docs/backlog/core/client-held-fd-bound.md`.
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
/// [`MAX_TIMELINES_PER_CLIENT`]).
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
    /// Live timeline objects per client. An entry exists only while the
    /// client holds at least one; claimed in `dispatch.rs` before the import
    /// is delegated, released by its destruction hook -- the same pairing
    /// `wl_buffers.rs` documents, including the one phantom unit a failed
    /// import leaves on an already-dead client.
    timelines: HashMap<ClientId, u32>,
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

    /// Counts one more live timeline for `client`, answering whether the
    /// import must be refused (past [`MAX_TIMELINES_PER_CLIENT`]). Called
    /// before the import is delegated, so a refused one is never counted.
    fn claim_timeline(&mut self, client: &ClientId) -> bool {
        let live = self.timelines.entry(client.clone()).or_insert(0);
        if *live >= MAX_TIMELINES_PER_CLIENT {
            return true;
        }
        *live = live.saturating_add(1);
        false
    }

    /// Forgets one live timeline for the client a destroyed timeline object
    /// belonged to. Also what drains a disconnect, whose cleanup destroys
    /// every object.
    fn forget_timeline(&mut self, client: &ClientId) {
        if let Some(live) = self.timelines.get_mut(client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.timelines.remove(client);
            }
        }
    }

    /// How many live timelines `client` holds.
    fn timelines_for(&self, client: &ClientId) -> u32 {
        self.timelines.get(client).copied().unwrap_or(0)
    }

    /// How many timelines every client holds between them. Test-only.
    #[cfg(test)]
    pub(crate) fn timelines_in_flight(&self) -> u32 {
        self.timelines.values().sum()
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
/// per-client live-timeline cap, or past its pressure grace while the fd
/// table is pressured, with the protocol's own `invalid_timeline` on the
/// manager. Answers `true` when it refused (the caller then does not
/// delegate).
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
    let Some(wp_linux_drm_syncobj_manager_v1::Request::ImportTimeline { .. }) =
        (request as &dyn Any).downcast_ref::<wp_linux_drm_syncobj_manager_v1::Request>()
    else {
        return false;
    };
    let id = client.id();
    if super::dispatch::pressure_refusal(
        state.drm_syncobj.timelines_for(&id),
        PRESSURE_GRACE_TIMELINES,
    ) {
        resource.post_error(
            wp_linux_drm_syncobj_manager_v1::Error::InvalidTimeline,
            format!(
                "timeline refused: compositor-wide file-descriptor pressure, and this client \
                 holds more than the {PRESSURE_GRACE_TIMELINES}-timeline pressure grace"
            ),
        );
        return true;
    }
    if !state.drm_syncobj.claim_timeline(&id) {
        return false;
    }
    tracing::debug!(
        max = MAX_TIMELINES_PER_CLIENT,
        "refusing a syncobj timeline past the per-client live count (protocol error)"
    );
    resource.post_error(
        wp_linux_drm_syncobj_manager_v1::Error::InvalidTimeline,
        format!(
            "timeline refused: this client already holds the maximum of \
             {MAX_TIMELINES_PER_CLIENT} live timelines"
        ),
    );
    true
}

/// The destruction half of [`reject_excess_timeline`]'s count, and the
/// explicit-buffer set's pruning: called from `dispatch.rs`'s `destroyed`
/// for every object, folding away for all but the two interfaces it reads.
pub(super) fn forget_destroyed<I>(state: &mut State, client: &ClientId, resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_timeline_v1;
    use smithay::reexports::wayland_server::protocol::wl_buffer;
    if TypeId::of::<I::Request>() == TypeId::of::<wp_linux_drm_syncobj_timeline_v1::Request>() {
        state.drm_syncobj.forget_timeline(client);
    } else if TypeId::of::<I::Request>() == TypeId::of::<wl_buffer::Request>()
        && !state.drm_syncobj.explicit.is_empty()
    {
        state.drm_syncobj.explicit.forget(&resource.id());
    }
}
