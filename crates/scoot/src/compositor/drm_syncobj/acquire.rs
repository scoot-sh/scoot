//! The acquire half of explicit sync: hold a commit until its acquire point
//! signals.
//!
//! [`pre_commit`] runs as a pre-commit hook on every surface (installed by
//! `CompositorHandler::new_surface` only while the global exists). For a
//! commit Smithay will accept -- a new dma-buf with both points, the release
//! point after the acquire point on a shared timeline -- whose acquire point
//! has not signalled yet, it registers an eventfd on that point
//! (`DrmSyncPoint::generate_blocker`) as a calloop source and adds a blocker
//! to the commit's transaction. Smithay then holds that transaction (and any
//! later one touching the same surface) until the blocker releases; other
//! surfaces, and every other client, carry on. When the eventfd fires, the
//! source removes itself and asks Smithay to apply whatever became ready
//! (`blocker_cleared`).
//!
//! What anvil's version of this (the pinned rev's `shell/mod.rs`) leaves out,
//! and this one does not:
//!
//! - **An already-signalled point costs one ioctl, not an eventfd.** Checked
//!   with `Fence::is_signaled` (a timeline query) first: no eventfd, no
//!   source, no wakeup, and nothing allocated here.
//! - **A surface destroyed mid-wait takes its sources with it**
//!   ([`forget_surface`], from `CompositorHandler::destroyed`, which Smithay
//!   also runs for every surface of a disconnecting client). Anvil leaves the
//!   source registered until the point signals, which for a point that never
//!   signals is an eventfd and a calloop source for the life of the process
//!   -- and since Smithay never destroys an imported syncobj handle (see the
//!   parent module), the syncobj it waits on never goes away either. The
//!   blockers of a destroyed surface are released rather than left pending
//!   ([`AcquireBlocker`]), so the client's transaction queue does not keep a
//!   dead transaction forever; a transaction Smithay applies with a dead
//!   surface in it skips that surface.
//! - **Outstanding waits are bounded per client**
//!   ([`MAX_ACQUIRE_WAITS_PER_CLIENT`](super::MAX_ACQUIRE_WAITS_PER_CLIENT),
//!   and a smaller grace while the fd table is pressured). Past it the client
//!   is disconnected with `wl_display.no_memory`: the syncobj surface's own
//!   error enum describes malformed requests, and none of them is this --
//!   "the compositor will not hold more of your resources" is exactly what
//!   `no_memory` says, and it is the code libwayland servers send when a
//!   client's demands cannot be met. A pre-commit hook cannot refuse a commit
//!   (it has no return value), so a kill is the only refusal there is; a
//!   silent skip of the wait would scan out unfinished buffers.
//!
//! A point that never signals therefore stalls exactly one surface: its
//! transaction stays queued, the surface keeps showing its previous buffer,
//! and the client's other surfaces and every other client are untouched
//! (Smithay's queue orders transactions per surface, not per client). The
//! stall ends when the point signals, the surface is destroyed or the client
//! disconnects.
//!
//! **Session pause.** Nothing here pauses with the session: syncobj ioctls
//! need no DRM master, so an eventfd registered before a VT switch still
//! fires while switched away, and the commit applies then (nothing renders
//! until the switch back, which composites whatever is current).
//!
//! **Allocation.** The already-signalled path and the hook's own
//! bookkeeping allocate nothing per commit (the per-surface record and its
//! `Vec` are created once and reused). A commit that really waits pays what
//! Smithay's API makes unavoidable: `generate_blocker` creates an eventfd and
//! two `Arc`s, calloop boxes the source, and `add_blocker` boxes the blocker.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::backend::renderer::sync::Fence;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::wayland::compositor::{
    Blocker, BlockerState, BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker,
    with_states,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::drm_syncobj::{DrmSyncPoint, DrmSyncPointBlocker, DrmSyncobjCachedState};

use super::{MAX_ACQUIRE_WAITS_PER_CLIENT, PRESSURE_GRACE_ACQUIRE_WAITS};
use crate::compositor::{State, no_memory};

/// Outstanding acquire waits, per surface and per client.
#[derive(Debug, Default)]
pub(crate) struct Waits {
    /// Outstanding waits per client. An entry exists only while the client
    /// has at least one.
    per_client: HashMap<ClientId, u32>,
    /// The surfaces that have waited, with their outstanding sources. An
    /// entry is created on a surface's first wait and kept (with its `Vec`'s
    /// capacity and its `abandoned` flag) until the surface is destroyed, so
    /// a client waiting every frame allocates nothing here after the first.
    per_surface: HashMap<ObjectId, SurfaceWaits>,
    /// Distinguishes one wait's source from another's on the same surface.
    /// `u64`: one per blocked commit, never wraps in practice.
    next_key: u64,
    /// Whether a failure to set up a wait has been warned about already, so
    /// a persistent failure (an exhausted fd table) warns once, not per
    /// commit.
    warned_setup_failure: bool,
}

/// One surface's outstanding waits.
#[derive(Debug)]
struct SurfaceWaits {
    /// Whose count these waits are claimed against, and whose queue is
    /// re-run when the surface goes.
    client: Client,
    /// Set when the surface is destroyed: every blocker this surface ever
    /// added then reports released (see [`AcquireBlocker`]). Shared by all
    /// of the surface's blockers, so a new wait clones an `Arc` rather than
    /// allocating one.
    abandoned: Arc<AtomicBool>,
    /// `(key, source)` for every wait still outstanding.
    pending: Vec<(u64, RegistrationToken)>,
}

impl Waits {
    /// How many waits `client` has outstanding.
    fn live_for(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).copied().unwrap_or(0)
    }

    /// Forgets one outstanding wait for `client`.
    fn release_one(&mut self, client: &ClientId) {
        if let Some(live) = self.per_client.get_mut(client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.per_client.remove(client);
            }
        }
    }

    /// A wait's eventfd fired: its source is removing itself (calloop drops
    /// it on `PostAction::Remove`), so only the bookkeeping goes.
    fn finished(&mut self, surface: &ObjectId, key: u64) {
        let Some(entry) = self.per_surface.get_mut(surface) else {
            return;
        };
        let Some(at) = entry.pending.iter().position(|(k, _)| *k == key) else {
            return;
        };
        entry.pending.swap_remove(at);
        let client = entry.client.id();
        self.release_one(&client);
    }

    /// How many waits every client has outstanding. Test-only.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> u32 {
        self.per_client.values().sum()
    }

    /// How many surfaces have a wait record. Test-only.
    #[cfg(test)]
    pub(crate) fn surfaces_tracked(&self) -> usize {
        self.per_surface.len()
    }

    /// The sources a surface's outstanding waits are registered as.
    /// Test-only: lets a test ask the event loop whether they still exist.
    #[cfg(test)]
    pub(crate) fn tokens_for(&self, surface: &WlSurface) -> Vec<RegistrationToken> {
        self.per_surface
            .get(&surface.id())
            .map(|entry| entry.pending.iter().map(|(_, token)| *token).collect())
            .unwrap_or_default()
    }
}

/// Smithay's blocker for one acquire point, released early if its surface
/// is destroyed.
///
/// Without the second half, removing the source of a destroyed surface (so
/// that a point which never signals does not hold an eventfd forever) would
/// leave the blocker pending forever too, and with it a dead transaction in
/// the client's queue that Smithay rescans on every later commit.
struct AcquireBlocker {
    inner: DrmSyncPointBlocker,
    abandoned: Arc<AtomicBool>,
}

impl Blocker for AcquireBlocker {
    fn state(&self) -> BlockerState {
        if self.abandoned.load(Ordering::Acquire) {
            BlockerState::Released
        } else {
            self.inner.state()
        }
    }
}

/// The pre-commit hook: classifies the commit's buffer for the release hold
/// and, when its acquire point is still pending, holds the commit until it
/// signals. See the module doc.
///
/// Runs before Smithay's own syncobj hook (this one is added when the
/// surface is created, Smithay's when the client asks for a syncobj
/// surface), so it only waits on a commit that hook will accept; a
/// malformed one gets no eventfd and is then killed by Smithay's error.
pub(crate) fn pre_commit(state: &mut State, dh: &DisplayHandle, surface: &WlSurface) {
    let explicit = &mut state.drm_syncobj.explicit;
    let Some(acquire) = with_states(surface, |states| {
        let mut attributes = states.cached_state.get::<SurfaceAttributes>();
        let Some(BufferAssignment::NewBuffer(buffer)) = attributes.pending().buffer.as_ref() else {
            return None;
        };
        let mut points = states.cached_state.get::<DrmSyncobjCachedState>();
        let points = points.pending();
        let acquire = match (&points.acquire_point, &points.release_point) {
            (Some(acquire), Some(release))
                if ordered(acquire, release) && get_dmabuf(buffer).is_ok() =>
            {
                Some(acquire.clone())
            }
            _ => None,
        };
        explicit.note(buffer, acquire.is_some());
        acquire
    }) else {
        return;
    };
    if acquire.is_signaled() {
        return;
    }
    let Some(client) = surface.client() else {
        return;
    };
    let client_id = client.id();
    let waits = &mut state.drm_syncobj.waits;
    let live = waits.live_for(&client_id);
    if live >= MAX_ACQUIRE_WAITS_PER_CLIENT
        || crate::compositor::dispatch::pressure_refusal(live, PRESSURE_GRACE_ACQUIRE_WAITS)
    {
        tracing::debug!(
            live,
            max = MAX_ACQUIRE_WAITS_PER_CLIENT,
            "disconnecting a client past its outstanding acquire-wait bound"
        );
        refuse_wait(dh, &client, live);
        return;
    }
    let (blocker, source) = match acquire.generate_blocker() {
        Ok(pair) => pair,
        Err(error) => {
            setup_failed(waits, &error);
            return;
        }
    };
    let key = waits.next_key;
    waits.next_key = waits.next_key.wrapping_add(1);
    let surface_id = surface.id();
    let entry = waits
        .per_surface
        .entry(surface_id.clone())
        .or_insert_with(|| SurfaceWaits {
            client: client.clone(),
            abandoned: Arc::new(AtomicBool::new(false)),
            pending: Vec::new(),
        });
    let abandoned = Arc::clone(&entry.abandoned);
    // A `Client` handle, not just its id: `blocker_cleared` needs one, and
    // wayland-server has no id-to-client lookup. The source (and so this
    // handle) lives only until the point signals or the surface is
    // destroyed, whichever is first.
    let fired_client = client.clone();
    let inserted = state
        .loop_handle
        .insert_source(source, move |(), &mut (), state| {
            state.drm_syncobj.waits.finished(&surface_id, key);
            apply_ready(state, &fired_client);
            Ok(())
        });
    match inserted {
        Ok(token) => {
            entry.pending.push((key, token));
            *waits.per_client.entry(client_id).or_insert(0) += 1;
            // The wait's eventfd, counted against fd pressure's cached table
            // reading (see `fd_pressure::table`).
            crate::compositor::fd_pressure::note_opened(1);
            add_blocker(
                surface,
                AcquireBlocker {
                    inner: blocker,
                    abandoned,
                },
            );
        }
        Err(error) => setup_failed(waits, &error.error),
    }
}

/// Disconnects `client` with `wl_display.error(no_memory)` for going past its
/// acquire-wait bound. See the module doc for the choice of code, and
/// [`no_memory::disconnect`] for how it is posted.
fn refuse_wait(dh: &DisplayHandle, client: &Client, live: u32) {
    no_memory::disconnect(
        dh,
        client,
        format!(
            "explicit sync: this client already has {live} commits waiting on unsignalled \
             acquire points (the bound is {MAX_ACQUIRE_WAITS_PER_CLIENT}, \
             {PRESSURE_GRACE_ACQUIRE_WAITS} under file-descriptor pressure)"
        ),
    );
}

/// Whether `release` may follow `acquire`: Smithay posts
/// `conflicting_points` when both sit on one timeline and the release point
/// is not after the acquire point, so such a commit is never waited on.
fn ordered(acquire: &DrmSyncPoint, release: &DrmSyncPoint) -> bool {
    acquire.timeline() != release.timeline() || release.point() > acquire.point()
}

/// A wait could not be set up (no eventfd, no epoll registration). The
/// commit then applies unsynchronised -- a possibly unfinished frame for
/// that client, never a stall or a kill for something that is the
/// compositor's resource problem, not the client's. Warned once.
fn setup_failed(waits: &mut Waits, error: &dyn std::fmt::Display) {
    if waits.warned_setup_failure {
        tracing::debug!(%error, "explicit sync: could not wait on an acquire point");
    } else {
        waits.warned_setup_failure = true;
        tracing::warn!(
            %error,
            "explicit sync: could not wait on an acquire point; the commit applies without \
             waiting (further failures are logged at debug)"
        );
    }
}

/// Asks Smithay to apply whatever of `client`'s queued transactions is now
/// ready. Harmless for a client that has gone: Smithay skips every surface
/// of a transaction that no longer exists.
fn apply_ready(state: &mut State, client: &Client) {
    let dh = state.display_handle.clone();
    state
        .client_compositor_state(client)
        .blocker_cleared(state, &dh);
}

/// A surface is gone (destroyed, or its client disconnected): removes every
/// eventfd source it still has registered, releases its blockers, and hands
/// the client's count back. Called from `CompositorHandler::destroyed`.
///
/// If any wait was outstanding and the client is still connected, the
/// client's queue is re-run from an idle callback rather than here: this
/// runs inside a destruction hook, and applying transactions from there
/// would re-enter the commit handler mid-destruction.
pub(crate) fn forget_surface(state: &mut State, surface: &WlSurface) {
    if !state.drm_syncobj.active() {
        return;
    }
    let waits = &mut state.drm_syncobj.waits;
    let Some(entry) = waits.per_surface.remove(&surface.id()) else {
        return;
    };
    entry.abandoned.store(true, Ordering::Release);
    if entry.pending.is_empty() {
        return;
    }
    let client_id = entry.client.id();
    for (_, token) in &entry.pending {
        state.loop_handle.remove(*token);
        waits.release_one(&client_id);
    }
    let client = entry.client;
    state
        .loop_handle
        .insert_idle(move |state| apply_ready(state, &client));
}
