//! Seat reconnect: a scoot-side second chance after the seat daemon loses
//! the VT-switch-back master race.
//!
//! Background: on a VT switch back, seatd re-acquires DRM master for the
//! session with exactly one `SET_MASTER` and never retries. If it loses the
//! race with the previous VT's owner, the client keeps a master-less fd for
//! the rest of the session: `reactivate` fails, the probe diagnoses
//! vacant-but-untakeable (`EACCES`), and every output reports `live: false`.
//! A same-client re-open cannot fix that -- seatd hands back the *same*
//! master-less file (see
//! `docs/backlog/resolved/seatd-reacquire-after-vt-switch-done.md`). What
//! can fix it is a *new* seat client: dropping the seat connection frees
//! seatd's per-client entry, so a fresh connect-plus-open is first-to-open
//! and seatd-as-root takes vacant master while handing the fd over.
//!
//! That is what this module does, once per `ActivateSession`, but only on
//! the vacant diagnosis. Proven live on the Asahi M2 (greeter-on-tty1 case
//! and no-greeter case, plus a root control): every lost race in the matrix
//! recovered, with the input gap measured in the tens of milliseconds --
//! see the ticket for the numbers.
//!
//! Shape of one attempt (all inside an event-loop idle, never a spin):
//! drop the heads first (teardown-first: the old pools' framebuffers would
//! fail the new `DrmDevice`'s snapshot -- the parent ticket measured
//! `MODE_OBJ_GETPROPERTIES` `EINVAL` on our own dumb framebuffer), drop the
//! seat connection (removing the session notifier closes it, which frees
//! seatd's entry), connect fresh, open the same node, rebuild the device
//! and its heads, re-init libinput on the new session, rewire the loop,
//! re-bind the existing outputs, and prove master synchronously
//! (`pause` + `activate` runs `reset_state`'s atomic commit -- the same
//! commit the normal path proves with) before reporting `live: true`.
//! Anything that fails short of a built device falls back to rebuilding
//! the heads on the *old* device -- or, when the old device itself drives
//! nothing any more (every display unplugged while switched away), to
//! keeping the old wiring dark with no heads at all: today's `live: false`
//! with a working keyboard and future retries, never a stranded session
//! and never a process exit. Only a dead seat itself (the fresh connect
//! failing, which dooms the old connection too) exits, through the same
//! supervisor-restarts-into-a-fresh-session contract as any other seat
//! loss.
//!
//! Both tiers rebuild through the same shape. On the CPU tier the heads
//! carry dumb pools and surfaces; on the GPU tier they carry a
//! `DrmCompositor` each, and the fresh build carries a renderer per head
//! that is handed to the output's existing render backend through
//! `Backend::replace_scanout` -- never a new output, never a mixed tier.
//! A fresh device that comes up on the other tier than the session runs is
//! not a recovery but a tier switch, so it restores the old device instead.
//! In-flight flips need no refuse-and-retry here: the pause path already
//! discarded the CPU tier's numbers and drained the GPU tier's queue in
//! `reactivate` before this idle runs, the old DRM registration is removed
//! before the old backend drops (so no late completion can settle a new
//! head early), and a `DrmCompositor` queues behind a pending flip rather
//! than refusing it -- dropping one with frames queued releases its client
//! buffers unwaited, exactly like the pause path does.
//!
//! Hard edges, as the ticket pins them: held-vs-vacant discrimination
//! unchanged (this acts only on `LostVacant`); privileged self-recovery
//! untouched (it never reaches here); `live: false` until the synchronous
//! proof passes; one attempt per `ActivateSession`, event-driven via an
//! idle, never a spin; input re-initialised on the new session with the
//! swap gap measured and logged; no allocation on any per-frame path
//! (everything here runs once per VT switch, on the calloop thread).

use std::time::Instant;

use scoot_core::OutputId;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::Session;
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::connector;
use smithay::reexports::input::Libinput;

use super::hotplug;
use super::stale_vblanks::StaleVblanks;
use super::{Reactivation, State, Tty};
use crate::cli::RendererKind;
use crate::compositor::output_config::ModeRequests;

/// Whether `outcome` earns a reconnect attempt right now. Only the vacant
/// shape does: held master belongs to a live session (reconnecting would
/// abandon a device that is rightfully busy), and every other failure is
/// something a fresh fd cannot fix. One attempt per `ActivateSession`,
/// event-driven, never a spin: each activation arms at most one idle (see
/// `arm`), and each idle runs exactly one attempt.
pub(super) fn should_attempt(outcome: Reactivation) -> bool {
    matches!(outcome, Reactivation::LostVacant)
}

/// Arms this activation's one reconnect attempt: an event-loop idle that
/// runs `run` once dispatch settles. Called with every non-recovered
/// activation diagnosis, not just the vacant one: the first vacant
/// activation arms the idle and snapshots its diagnosis, and a later
/// activation before the idle runs overwrites the snapshot without
/// arming a second idle -- so a queued idle never acts on a superseded
/// diagnosis (see `run`'s re-check). Called with the vacant diagnosis
/// already established; re-checks nothing itself.
pub(super) fn arm(state: &mut State, outcome: Reactivation) {
    let Some(tty) = state.tty.as_mut() else {
        return;
    };
    if let Some(stored) = tty.reconnect_armed.as_mut() {
        // Already armed: record the latest diagnosis so `run` can stand
        // down on a superseded one. No second idle: exactly one attempt
        // per arming window, event-driven, never a spin.
        *stored = outcome;
        return;
    }
    if !should_attempt(outcome) {
        return;
    }
    tty.reconnect_armed = Some(outcome);
    tracing::info!(
        "seat reconnect armed: the seat daemon lost the master race on a vacant device; \
         trying one fresh seat client"
    );
    state.loop_handle.insert_idle(run);
}

/// Whether a queued attempt may still proceed: the diagnosis it armed on
/// must still be the vacant one, the session must not have recovered
/// under it, and the session must not have been switched away again
/// meanwhile. A pure predicate so the N1/N2 skip rules are pinned by
/// headless tests -- the fd half needs real seat hardware, this half
/// must not rot silently.
fn should_proceed(diagnosis: Reactivation, active: bool, session_paused: bool) -> bool {
    diagnosis == Reactivation::LostVacant && !active && !session_paused
}

/// One armed attempt. See the module doc for the shape; each numbered step
/// below names its fallback, so a reader can check that no failure leaves
/// the session worse than today's `live: false`.
fn run(state: &mut State) {
    let started = Instant::now();
    // The attempt is spent whatever happens below: taking first means an
    // early return cannot leave a stale arm behind, and a second activation
    // arms its own attempt afterwards.
    let wanted = state.renderer;
    let Some(tty) = state.tty.as_mut() else {
        return;
    };
    let Some(diagnosis) = tty.reconnect_armed.take() else {
        return;
    };
    // The single gate on every queued attempt (see `should_proceed`): a
    // superseded diagnosis, a recovery under the arm, or a switch-away
    // since the arm each stands the attempt down. The per-case lines below
    // only name the reason -- the proceed/skip truth lives in that one
    // predicate, pinned headlessly there.
    if !should_proceed(diagnosis, tty.active, tty.session_paused) {
        if diagnosis != Reactivation::LostVacant {
            // A second activation landed between the arm and this idle
            // with a different answer (held master, a recovery, any other
            // failure). Acting on the stale vacant one anyway would tear
            // down a backend whose race the follow-up already settled --
            // at best wasteful, at worst swapping in a fresh fd against a
            // held master.
            tracing::info!(
                ?diagnosis,
                "seat reconnect skipped: a later activation superseded the vacant diagnosis"
            );
        } else if tty.active {
            // The session recovered between the arm and this idle (a
            // second activation that won its own race): an active session
            // holds master, so there is nothing to reconnect. Tearing down
            // a healthy backend here would trade a live display for a
            // needless swap.
            tracing::info!(
                "seat reconnect skipped: the session re-acquired drm master before the attempt ran"
            );
        } else {
            // The session was switched away again before this idle ran:
            // rebuilding while paused would open on an inactive client
            // (refused), tear down a backend another activation will
            // settle, and wrongly clear `session_paused` on the swap path.
            // Stand down instead -- the next switch back arms its own
            // attempt.
            tracing::info!(
                "seat reconnect skipped: the session switched away again before the attempt ran"
            );
        }
        return;
    }
    // The session's tier decides the rebuild shape below: the CPU tier
    // swaps heads alone (its render backends hold no device state), while
    // the GPU tier additionally hands each fresh renderer to its output's
    // existing backend (see `run_scanout`). Resolved from the session's
    // own tier (`state.renderer`, settled at startup and never rewritten
    // afterwards), not from the heads: after an empty restore there are
    // no heads to read, and an empty session misreads as the CPU tier --
    // which would rebuild GPU heads and then drop them for want of a
    // binding, reporting `live: true` with nothing presenting. The same
    // `RendererKind` the hotplug path maps per head, so no `auto` request
    // state is needed here.
    let gpu_tier = wanted != RendererKind::Pixman;
    if gpu_tier {
        return run_scanout(state, started, wanted);
    }
    // Snapshot everything the rebuild needs while the old backend is whole.
    // Connectors plus their outputs, so the new heads re-bind to the
    // outputs the rest of the compositor already has.
    let bindings: Vec<(connector::Handle, OutputId)> = tty
        .heads
        .iter()
        .filter_map(|head| Some((head.connector, head.output?)))
        .collect();
    let path = tty.path.clone();
    let modes = tty.modes.clone();
    let session_token = tty.session_token;

    // Teardown first: drop the heads (pools, surfaces, flip state) before
    // the new `DrmDevice` is built, while the old device and session are
    // still in place. The old pools' framebuffers would fail the new
    // device's snapshot; the old device itself holds no framebuffers, so
    // only the heads have to go. Single-threaded dispatch means no event
    // can observe the headless interval.
    {
        let Some(tty) = state.tty.as_mut() else {
            return;
        };
        std::mem::take(&mut tty.heads);
    }

    // Drop the seat connection: removing the session notifier drops the
    // one strong `Rc` keeping libseat's client alive, which closes the
    // socket, which frees seatd's per-client device entry. Only then can
    // a fresh client's open be first-to-open. The DRM and libinput sources
    // stay registered across this: their fds are still open in the old
    // backend, nothing dispatches mid-idle, and keeping them is what lets
    // the restore path below reuse them untouched.
    state.loop_handle.remove(session_token);

    // A fresh seat client. Failure here means the seat itself is gone (the
    // socket the old connection lived on is unreachable), which dooms the
    // old connection too -- its next dispatch would fail the same way and
    // exit through the seat-loss contract. Exiting now, loudly, is that
    // contract arriving one event early, and the supervisor's fresh start
    // is the documented way back (it re-acquires master through the seat
    // daemon, which is exactly what this attempt wanted).
    let (mut session, notifier) = match LibSeatSession::new() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: no new seat connection after dropping the old one; \
                 the seat is gone, so stopping (a supervisor restarts into a fresh session, \
                 which re-acquires master through the seat daemon)"
            );
            std::process::exit(1);
        }
    };
    let session_came_up_active = session.is_active();
    tracing::info!(
        came_up_active = session_came_up_active,
        "seat reconnect: fresh seat client connected"
    );

    // Rebuild input on the new session before touching the old backend's
    // registration: failure below restores with the old input still wired.
    let input = match rebuild_input(&session) {
        Ok(input) => input,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: {}; restoring the old device",
                restore_guidance()
            );
            restore(state, session, notifier, &bindings, &modes, wanted);
            return;
        }
    };

    // The fresh open: first-to-open on the freed entry, so seatd-as-root
    // takes vacant master while handing the fd over. Teardown-first is
    // already done (step above), which is what lets `DrmDevice::new`
    // inside pass its framebuffer snapshot.
    //
    // The request the rebuild runs under follows the resolved tier (see
    // `open_device`): the dumb-tier gate above makes this `Cpu` today;
    // `scanout-seat-reconnect` extends the gate, and the `Gles` arm already
    // maps faithfully so it keeps rebuilding the tier the session runs.
    let request = match wanted {
        RendererKind::Pixman => crate::cli::RendererRequest::Cpu,
        RendererKind::Gles => crate::cli::RendererRequest::Gpu,
    };
    let device = match super::open_device(&mut session, &path, &modes, request) {
        Ok(device) => device,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: {}; restoring the old device",
                restore_guidance()
            );
            restore(state, session, notifier, &bindings, &modes, wanted);
            return;
        }
    };

    // Prove master synchronously, in the normal path's own shape: `pause`
    // then `activate` runs `reset_state`'s atomic commit, which only a
    // master-holding fd passes. `live` stays false until this succeeds.
    let mut device = device;
    device.drm.pause();
    match device.drm.activate(true) {
        Ok(()) => {
            swap(
                state,
                session,
                notifier,
                input,
                device,
                &bindings,
                SwapLive::Live,
                started,
                session_came_up_active,
            );
        }
        Err(error) => {
            // A fresh fd that still cannot modeset: the holder came back
            // between the open and the proof, or the device refused for its
            // own reasons. Keep the fresh device anyway -- unlike the old
            // poisoned fd it joins future races normally, so the next
            // switch back retries on a healthy fd.
            swap(
                state,
                session,
                notifier,
                input,
                device,
                &bindings,
                SwapLive::Dark {
                    proof: error.to_string(),
                },
                started,
                session_came_up_active,
            );
        }
    }
}

/// The GPU-tier attempt: the same teardown-first shape as [`run`], plus
/// the renderer handoff. Only compiled with the GPU tier present; without
/// it the tier cannot exist (the resolver keeps such a session on the CPU
/// tier), so reaching here is a loud decline rather than a rebuild.
///
/// The arm is already spent by [`run`] (it took the snapshot before
/// dispatching here), so this never re-takes it. Every failure short of a
/// dead seat restores the old device through [`restore_scanout`]; only a
/// dead seat exits, through the same contract [`run`] documents.
#[cfg(feature = "gpu-scanout")]
fn run_scanout(state: &mut State, started: Instant, wanted: RendererKind) {
    // Snapshot everything the rebuild needs while the old backend is whole.
    // Connectors plus their outputs, so the new heads re-bind to the
    // outputs the rest of the compositor already has.
    let (bindings, path, modes, session_token) = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished before a GPU-tier reconnect with no dispatch between"
            );
            tracing::error!(
                "seat reconnect: lost the old backend before the attempt; keeping the session"
            );
            return;
        };
        // One tier per session: every head shares it, so the first head's
        // answer is the session's. A head with no output never presents,
        // but it still drives its connector and must be rebuilt -- so the
        // bindings snapshot keeps the output-claimed shape the CPU path
        // rebinds by, while the teardown below drops every head.
        let bindings: Vec<(connector::Handle, OutputId)> = tty
            .heads
            .iter()
            .filter_map(|head| Some((head.connector, head.output?)))
            .collect();
        (
            bindings,
            tty.path.clone(),
            tty.modes.clone(),
            tty.session_token,
        )
    };

    // Teardown first, exactly like the CPU path: drop every head
    // (compositors, swapchains, flip state) before the new `DrmDevice` is
    // built, while the old device and session are still in place. The old
    // swapchain's framebuffers would fail the new device's snapshot the
    // way the old pools' would; the render backends stay -- their renderer
    // holds no framebuffer, only an EGL context and exported dma-bufs --
    // until the swap hands them their fresh renderers. Single-threaded
    // dispatch means no event can observe the headless interval.
    {
        let Some(tty) = state.tty.as_mut() else {
            return;
        };
        // No refuse-and-retry on queued frames: the pause path released the
        // GPU tier's held buffers and `reactivate` drained its pending frame
        // before this idle ran, and a compositor queues behind a pending
        // flip rather than refusing it -- so there is nothing to wait for.
        // The check below pins that reading: a head that ever reports one
        // in flight here is a future presenter shape this path has not been
        // taught, and standing down is safer than tearing down under it.
        if tty.heads.iter().any(|head| head.presenter.flip_in_flight()) {
            tracing::info!(
                "seat reconnect skipped: a flip is still in flight on the GPU tier; \
                 the next switch back retries"
            );
            return;
        }
        std::mem::take(&mut tty.heads);
    }

    // Drop the seat connection: removing the session notifier drops the
    // one strong `Rc` keeping libseat's client alive, which closes the
    // socket, which frees seatd's per-client device entry. Only then can
    // a fresh client's open be first-to-open. The DRM and libinput sources
    // stay registered across this: their fds are still open in the old
    // backend, nothing dispatches mid-idle, and keeping them is what lets
    // the restore path below reuse them untouched.
    state.loop_handle.remove(session_token);

    // A fresh seat client. Failure here means the seat itself is gone (the
    // socket the old connection lived on is unreachable), which dooms the
    // old connection too -- its next dispatch would fail the same way and
    // exit through the seat-loss contract. Exiting now, loudly, is that
    // contract arriving one event early, and the supervisor's fresh start
    // is the documented way back (it re-acquires master through the seat
    // daemon, which is exactly what this attempt wanted).
    let (mut session, notifier) = match LibSeatSession::new() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: no new seat connection after dropping the old one; \
                 the seat is gone, so stopping (a supervisor restarts into a fresh session, \
                 which re-acquires master through the seat daemon)"
            );
            std::process::exit(1);
        }
    };
    let session_came_up_active = session.is_active();
    tracing::info!(
        came_up_active = session_came_up_active,
        "seat reconnect: fresh seat client connected"
    );

    // Rebuild input on the new session before touching the old backend's
    // registration: failure below restores with the old input still wired.
    let input = match rebuild_input(&session) {
        Ok(input) => input,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: {}; restoring the old device",
                restore_guidance()
            );
            restore_scanout(state, session, notifier, &bindings, &modes, wanted);
            return;
        }
    };

    // The fresh open: first-to-open on the freed entry, so seatd-as-root
    // takes vacant master while handing the fd over. Teardown-first is
    // already done (step above), which is what lets `DrmDevice::new`
    // inside pass its framebuffer snapshot.
    let device = match super::open_device(&mut session, &path, &modes, wanted) {
        Ok(device) => device,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect failed: {}; restoring the old device",
                restore_guidance()
            );
            restore_scanout(state, session, notifier, &bindings, &modes, wanted);
            return;
        }
    };
    // One tier per session: a fresh device that fell back to the CPU tier
    // is not a recovery but a tier switch, so it restores the old device
    // instead of mixing tiers (a CPU head in a GPU session would be
    // rebuilt as the offscreen pipeline the tier choice exists to avoid).
    if !fresh_tier_matches(device.renderer, wanted) {
        tracing::error!(
            "seat reconnect failed: the fresh device came up on the CPU tier while this \
             session runs on the GPU tier; {}; restoring the old device",
            restore_guidance()
        );
        restore_scanout(state, session, notifier, &bindings, &modes, wanted);
        return;
    }

    // Prove master synchronously, in the normal path's own shape: `pause`
    // then `activate` runs `reset_state`'s atomic commit, which only a
    // master-holding fd passes. `live` stays false until this succeeds.
    let mut device = device;
    device.drm.pause();
    match device.drm.activate(true) {
        Ok(()) => {
            swap_scanout(
                state,
                session,
                notifier,
                input,
                device,
                &bindings,
                SwapLive::Live,
                started,
                session_came_up_active,
            );
        }
        Err(error) => {
            // A fresh fd that still cannot modeset: the holder came back
            // between the open and the proof, or the device refused for its
            // own reasons. Keep the fresh device anyway -- unlike the old
            // poisoned fd it joins future races normally, so the next
            // switch back retries on a healthy fd.
            swap_scanout(
                state,
                session,
                notifier,
                input,
                device,
                &bindings,
                SwapLive::Dark {
                    proof: error.to_string(),
                },
                started,
                session_came_up_active,
            );
        }
    }
}

/// Without the GPU tier the session cannot be on it (the resolver keeps
/// such a session on the CPU tier), so a GPU-tier attempt there declines
/// loudly before any teardown -- keeping today's path whole. This is the
/// only `run_scanout` a build without the feature has.
#[cfg(not(feature = "gpu-scanout"))]
fn run_scanout(state: &mut State, started: Instant, wanted: RendererKind) {
    let _ = (state, started, wanted);
    tracing::info!(
        "seat reconnect not attempted: this session is on the GPU tier, \
         which this build does not carry; keeping live:false until a \
         later switch back wins the race"
    );
}

/// Whether the fresh device came up on the tier the session runs: the
/// reconnect must never mix tiers. A pure predicate so headless tests pin
/// it -- the fd half needs real seat hardware, this half must not rot
/// silently. Only the GPU-tier attempt calls it outside tests, so it is
/// gated on either.
#[cfg(any(test, feature = "gpu-scanout"))]
fn fresh_tier_matches(fresh: RendererKind, wanted: RendererKind) -> bool {
    fresh == wanted
}

/// Whether the swapped-in device proved master and may report `live: true`.
enum SwapLive {
    /// The synchronous proof passed: master is held, re-probe and render.
    Live,
    /// The proof failed: keep the fresh device dark (it still joins
    /// future races normally) with today's message. Carries the proof's
    /// own error for the log line.
    Dark { proof: String },
}

/// Installs a built device: removes the old DRM and input registrations,
/// drops the old backend, wires the new sources, binds the existing
/// outputs to the new heads, and -- only on [`SwapLive::Live`] -- re-runs
/// the while-away re-probe and asks for the frame the modeset rides on.
#[allow(clippy::too_many_arguments)]
fn swap(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    input: (Libinput, LibinputInputBackend),
    device: super::Device,
    bindings: &[(connector::Handle, OutputId)],
    live: SwapLive,
    started: Instant,
    session_came_up_active: bool,
) {
    let (context, backend) = input;
    let super::Device {
        drm,
        notifier: drm_notifier,
        heads,
        renderer: _,
        reason: _,
        driver: _,
        gl_renderer: _,
    } = device;
    let device_id = drm.device_id();

    // Read the old registrations and the carried-over state first, while
    // the old backend is still in place. `None` here is unreachable
    // single-threaded: `state.tty` is set once at init and taken only in
    // this swap, with no dispatch interleaving -- so this is a
    // `debug_assert` plus an early return, not a process exit. (A
    // compositor exit takes every client's unsaved state with it; a loud
    // return on an unreachable branch strictly dominates it.) The fresh
    // pieces drop on return; the session source was already removed in
    // `run`, so session events are dead from here in the impossible case
    // -- still alive for IPC, still retryable by restart, never a kill.
    let (old_drm_token, old_libinput_token, path, modes) = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-swap with no dispatch between"
            );
            tracing::error!(
                "seat reconnect: lost the old backend mid-swap; keeping the old session"
            );
            return;
        };
        (
            tty.drm_token,
            tty.libinput_token,
            tty.path.clone(),
            tty.modes.clone(),
        )
    };
    // Remove the old registrations while the old backend still owns its
    // fds, so no fd number can be reused underneath the poll registry;
    // then drop the old backend whole (its `pause` plus closing the old
    // fds, after their loop registrations are already gone).
    let input_gap = Instant::now();
    state.loop_handle.remove(old_drm_token);
    state.loop_handle.remove(old_libinput_token);
    // Cannot fail after the read above: no source dispatches between the
    // two, single-threaded, so the backend observed `Some` a moment ago is
    // still `Some`. Asserted, not exited -- see above for why no
    // `process::exit` lives on this path any more.
    debug_assert!(
        state.tty.is_some(),
        "tty vanished mid-swap with no dispatch between"
    );
    state.tty.take();
    // Wire the new sources. An insert refuses only while the loop is
    // shutting down (nothing else refuses one) -- and during a shutdown an
    // explicit `process::exit(1)` would misreport a clean teardown as a
    // crash to the supervisor, costing a restart-failure mark for nothing.
    // So a refusal logs and returns with the old backend already dropped:
    // the session runs backend-less until the shutdown completes, which is
    // exactly where it was headed anyway.
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the session during shutdown");
            return;
        }
    };
    let drm_token = match state
        .loop_handle
        .insert_source(drm_notifier, super::drm_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the drm device during shutdown");
            return;
        }
    };
    let libinput_token = match state
        .loop_handle
        .insert_source(backend, super::libinput_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register input during shutdown");
            return;
        }
    };
    let input_gap_ms = input_gap.elapsed().as_millis();
    // The new heads arrive unbound (`build_head` sets no output); the
    // scanout handoffs are spent here -- on the dumb tier they are empty
    // (a scanout session never reaches this swap; see `run`).
    let heads = heads
        .into_iter()
        .map(|(head, _scanout)| head)
        .collect::<Vec<_>>();
    let live_flag = matches!(live, SwapLive::Live);
    state.tty = Some(Tty {
        session,
        drm,
        heads,
        stale_vblanks: StaleVblanks::new(),
        device_id,
        modes,
        nothing_connected: false,
        libinput: context,
        active: live_flag,
        session_paused: false,
        path,
        session_token,
        drm_token,
        libinput_token,
        reconnect_armed: None,
    });
    rebind(state, bindings);
    match live {
        SwapLive::Live => {
            // What the normal reactivation arm does after a win: ask the
            // device what it says now (a display may have moved while the
            // session was away), apply it, and render the modeset's frame.
            // `None` is unreachable single-threaded (just wired above, no
            // dispatch between): assert plus a return that skips the render
            // rather than a process exit -- the fresh backend is already in
            // place and dark-safe, and the next frame or switch back heals
            // it.
            let outcomes = {
                let Some(tty) = state.tty.as_mut() else {
                    debug_assert!(state.tty.is_some(), "new backend vanished mid-swap");
                    tracing::error!(
                        "seat reconnect: lost the new backend mid-swap; skipping its re-probe"
                    );
                    return;
                };
                tty.reconfigure()
            };
            hotplug::apply(state, outcomes);
            state.request_render();
            state.update_cursor_hide(Instant::now());
            tracing::info!(
                came_up_active = session_came_up_active,
                input_gap_ms,
                total_ms = started.elapsed().as_millis(),
                "seat reconnect recovered the display: fresh seat client holds drm master"
            );
        }
        SwapLive::Dark { proof } => {
            tracing::error!(
                %proof,
                "seat reconnect opened a fresh device but cannot modeset on it: {}; \
                 keeping the fresh device dark (every output reports live:false) until a \
                 later switch back -- switch VTs away and back to retry the reactivation; \
                 if it never comes back, restart the session",
                keep_dark_guidance()
            );
        }
    }
}

/// Installs a built GPU-tier device: the same registration swap as
/// [`swap`], plus handing each fresh renderer to its output's existing
/// render backend. Only compiled with the GPU tier present.
///
/// The handoffs ride alongside the heads by connector: each fresh head's
/// connector names the output it re-binds to (see `rebind`), and the same
/// connector names the backend its renderer joins through
/// `Backend::replace_scanout`. A backend that is missing or not on the
/// GPU tier is a caller bug (one tier per session, outputs already bound)
/// -- asserted in debug builds, and turned dark loudly otherwise rather
/// than left mismatched: the fresh device stays, `live: false`, retryable
/// on the next switch back, exactly like [`SwapLive::Dark`].
#[cfg(feature = "gpu-scanout")]
#[allow(clippy::too_many_arguments)]
fn swap_scanout(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    input: (Libinput, LibinputInputBackend),
    device: super::Device,
    bindings: &[(connector::Handle, OutputId)],
    live: SwapLive,
    started: Instant,
    session_came_up_active: bool,
) {
    let (context, backend) = input;
    let super::Device {
        drm,
        notifier: drm_notifier,
        heads: built,
        renderer,
    } = device;
    debug_assert_eq!(
        renderer,
        RendererKind::Gles,
        "a GPU-tier swap was handed a CPU-tier device"
    );
    let device_id = drm.device_id();

    // The renderer halves, keyed by the connector their head drives, so
    // the handoff below cannot attach one screen's renderer to another's
    // backend. One small `Vec` on a path that runs once per VT switch,
    // matching `bindings`' own shape -- never on a per-frame path.
    let mut heads: Vec<super::head::Head> = Vec::with_capacity(built.len());
    let mut handoffs: Vec<(connector::Handle, crate::compositor::render::ScanoutHandoff)> =
        Vec::with_capacity(built.len());
    for (head, handoff) in built {
        handoffs.push((head.connector, handoff));
        heads.push(head);
    }

    let (old_drm_token, old_libinput_token, path, modes) = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-swap with no dispatch between"
            );
            tracing::error!(
                "seat reconnect: lost the old backend mid-swap; keeping the old session"
            );
            return;
        };
        (
            tty.drm_token,
            tty.libinput_token,
            tty.path.clone(),
            tty.modes.clone(),
        )
    };
    let input_gap = Instant::now();
    state.loop_handle.remove(old_drm_token);
    state.loop_handle.remove(old_libinput_token);
    debug_assert!(
        state.tty.is_some(),
        "tty vanished mid-swap with no dispatch between"
    );
    state.tty.take();
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the session during shutdown");
            return;
        }
    };
    let drm_token = match state
        .loop_handle
        .insert_source(drm_notifier, super::drm_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the drm device during shutdown");
            return;
        }
    };
    let libinput_token = match state
        .loop_handle
        .insert_source(backend, super::libinput_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register input during shutdown");
            return;
        }
    };
    let input_gap_ms = input_gap.elapsed().as_millis();
    let live_flag = matches!(live, SwapLive::Live);
    state.tty = Some(Tty {
        session,
        drm,
        heads,
        stale_vblanks: StaleVblanks::new(),
        device_id,
        modes,
        nothing_connected: false,
        libinput: context,
        active: live_flag,
        session_paused: false,
        path,
        session_token,
        drm_token,
        libinput_token,
        reconnect_armed: None,
    });
    // Bind the fresh heads to the outputs the compositor already has, by
    // the connector snapshot taken before teardown -- but unlike `rebind`,
    // without dropping the heads no binding covers yet (see below): an
    // empty-bindings retry after an empty restore, or a display that
    // arrived mid-switch, still needs outputs, and dropping them here
    // strands the session headless (a later re-probe with no heads plans
    // nothing) until restart.
    for (connector, id) in bindings {
        let connector = *connector;
        super::attach_where(
            state,
            move |head| head.connector == connector && head.output.is_none(),
            *id,
        );
    }
    remove_orphaned_outputs(state, bindings);
    state.reapply_output_power();
    // Hand each fresh renderer to its output's backend, by the connector
    // snapshot taken before teardown.
    let mut handoff_failed = false;
    for (connector, id) in bindings {
        let position = handoffs
            .iter()
            .position(|(head_connector, _)| *head_connector == *connector);
        let Some(position) = position else {
            continue;
        };
        let (_, handoff) = handoffs.remove(position);
        let Some(handoff) = handoff.backend else {
            continue;
        };
        let replaced = state
            .backends
            .get_mut(id)
            .is_some_and(|backend| backend.replace_scanout(handoff));
        if !replaced {
            tracing::error!(
                output = id.0,
                "seat reconnect: no GPU-tier backend to hand the fresh renderer to; \
                 keeping the fresh device dark until a later switch back retries it"
            );
            handoff_failed = true;
        }
    }
    // Heads no binding covers take the hotplug Added shape instead: a new
    // output each, attached by connector, with displaced windows restored
    // -- the same path a physical replug runs, and the reason an empty
    // restore stays retryable rather than terminal. A renderer whose head
    // is gone already (its connector vanished mid-switch) has no output
    // to present, so it drops here -- the backend keeps its old renderer,
    // which then has no presenter and draws nothing until the next switch
    // back.
    let mut added: Vec<hotplug::Change> = Vec::new();
    for (connector, handoff) in handoffs {
        let Some(head) = state
            .tty
            .as_ref()
            .and_then(|tty| tty.heads.iter().find(|head| head.connector == connector))
        else {
            continue;
        };
        if head.output.is_some() {
            continue;
        }
        let Some(renderer) = handoff.backend else {
            continue;
        };
        added.push(hotplug::Change::Added {
            connector,
            name: head.name.clone(),
            identity: head.identity.clone(),
            width: head.width,
            height: head.height,
            scanout: crate::compositor::render::ScanoutHandoff {
                backend: Some(renderer),
            },
        });
    }
    hotplug::apply(state, added);
    // Outputs left with no presenting head -- kept through an empty
    // restore whose retry re-added their connectors under fresh ids --
    // would otherwise linger headless: still laid out, still reporting
    // `live`, showing nothing. Remove them the way a hotplug would, so
    // their windows are adopted elsewhere. A no-op on every other path,
    // where every output presents (the last output is never removed:
    // `remove_output` refuses it, exactly as in `rebind`).
    let headless: Vec<OutputId> = state
        .outputs
        .iter_with_ids()
        .filter_map(|(id, _)| {
            let presents = state
                .tty
                .as_ref()
                .is_some_and(|tty| tty.heads.iter().any(|head| head.presents(id)));
            (!presents).then_some(id)
        })
        .collect();
    for id in headless {
        tracing::info!(
            output = id.0,
            "seat reconnect: removing an output no head presents after the rebuild"
        );
        if !state.remove_output(id) {
            tracing::error!(
                output = id.0,
                "seat reconnect: could not remove an output no head presents"
            );
        }
    }
    // A handoff failure is a dark fresh device, not a stranded one: mark
    // it the way the proof failure does and fall through to the `Dark`
    // message below. So is a fresh device with nothing presenting: the
    // proof passed, but no head survived onto an output (every output
    // creation refused) -- reporting `live: true` for that would lie to
    // every capture consumer checking the flag before trusting pixels.
    let nothing_presenting = state
        .tty
        .as_ref()
        .is_none_or(|tty| !tty.heads.iter().any(|head| head.output.is_some()));
    let live = if handoff_failed || nothing_presenting {
        if let SwapLive::Live = live {
            let proof = if handoff_failed {
                "the fresh renderers could not join their outputs".to_owned()
            } else {
                "no rebuilt head reached an output".to_owned()
            };
            SwapLive::Dark { proof }
        } else {
            live
        }
    } else {
        live
    };
    match live {
        SwapLive::Live => {
            let outcomes = {
                let Some(tty) = state.tty.as_mut() else {
                    debug_assert!(state.tty.is_some(), "new backend vanished mid-swap");
                    tracing::error!(
                        "seat reconnect: lost the new backend mid-swap; skipping its re-probe"
                    );
                    return;
                };
                tty.reconfigure()
            };
            hotplug::apply(state, outcomes);
            state.request_render();
            state.update_cursor_hide(Instant::now());
            tracing::info!(
                came_up_active = session_came_up_active,
                input_gap_ms,
                total_ms = started.elapsed().as_millis(),
                scanout = "gpu",
                "seat reconnect recovered the display: fresh seat client holds drm master"
            );
        }
        SwapLive::Dark { proof } => {
            tracing::error!(
                %proof,
                "seat reconnect opened a fresh device but cannot modeset on it: {}; \
                 keeping the fresh device dark (every output reports live:false) until a \
                 later switch back -- switch VTs away and back to retry the reactivation; \
                 if it never comes back, restart the session",
                keep_dark_guidance()
            );
        }
    }
}

/// The libinput half of a reconnect: a fresh context on `session` plus the
/// backend the loop will drive it through. Pure construction, no loop
/// interaction, so failure leaves every existing registration in place.
fn rebuild_input(session: &LibSeatSession) -> Result<(Libinput, LibinputInputBackend), String> {
    let interface = LibinputSessionInterface::from(session.clone());
    let mut context = Libinput::new_with_udev(interface);
    context
        .udev_assign_seat(&session.seat())
        .map_err(|()| "could not assign the seat to the new libinput context".to_owned())?;
    let backend = LibinputInputBackend::new(context.clone());
    Ok((context, backend))
}

/// Rebuilds heads on the *old* device after the fresh build (or the fresh
/// input) failed, and rewires the session and input on the new seat
/// client: today's `live: false` with a working keyboard, working
/// `change_vt`, and future retries -- never a stranded session and never a
/// process exit (a compositor exit takes every client's unsaved state with
/// it; see the N3 hardening).
///
/// When the old device itself drives nothing any more -- every display
/// unplugged while switched away, with the switch back losing the race in
/// the same cycle -- there are no heads to rebuild. That used to be a
/// `process::exit(1)`; it is now the same survival shape with an empty
/// head list: the old wiring stays dark, every output reports `live:
/// false` (all but the last are removed the way a hotplug would), the new
/// seat client still takes over session and input, and the next switch
/// back retries. Today's no-reconnect path survives exactly this window
/// (`live: false` + hotplug reconfiguration on the next win), so turning
/// it into a kill would be strictly worse.
///
/// The `state.tty is None` guards below are unreachable single-threaded
/// (`state.tty` is `Some` for the whole idle: set once at init, taken only
/// inside `swap`, where no dispatch interleaves) -- each is a
/// `debug_assert` plus a loud early return, never an exit. The loop-insert
/// refusals can only fire while the loop is shutting down (nothing else
/// refuses an insert): they log and return so a clean teardown is not
/// misreported as a crash to the supervisor.
fn restore(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    bindings: &[(connector::Handle, OutputId)],
    modes: &ModeRequests,
    wanted: RendererKind,
) {
    // Fresh probe, not the startup cache: the connectors may have moved
    // while the session was away, and the restore must see what is there
    // now (same freshness the hotplug path probes with). An unreadable
    // device probes as nothing-connected rather than exiting: the empty
    // survival below keeps the session alive for the retry.
    let connected = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        match tty.drm.resource_handles() {
            Ok(resources) => {
                super::gpu::find_all(&tty.drm, &resources, modes, super::gpu::Freshness::Reprobe)
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    "seat reconnect restore: cannot re-read the old device; keeping the session dark \
                     until a later switch back retries it"
                );
                Vec::new()
            }
        }
    };
    // The session's tier is settled (it drove these outputs minutes ago);
    // keep it rather than re-deciding: a head that cannot rejoin its own
    // tier is refused, exactly as at startup. The plan follows the tier
    // (no software rejection -- it is already decided), and the request
    // follows it too, so a refusal here logs the way the tier's own
    // startup path would.
    let (heads, _decided, _failures, _trying) = {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        let drm_fd = tty.drm.device_fd().clone();
        let request = match wanted {
            RendererKind::Pixman => crate::cli::RendererRequest::Cpu,
            RendererKind::Gles => crate::cli::RendererRequest::Gpu,
        };
        super::build_heads(
            &mut tty.drm,
            &drm_fd,
            connected,
            Some(wanted),
            crate::compositor::render::policy::device_plan_for_resolved_tier(wanted),
            request,
        )
    };
    let heads: Vec<super::head::Head> = heads.into_iter().map(|(head, _scanout)| head).collect();
    // Rewire input on the new session first: the old context's interface
    // holds the dropped (dead) session, so it cannot survive the swap.
    // The old input source is removed only once the replacement is built.
    // A rebuild failure keeps the old input wiring rather than exiting:
    // the old context may be dead with it, but the session stays alive
    // for IPC and the next switch back retries -- strictly better than a
    // kill.
    match rebuild_input(&session) {
        Ok((context, backend)) => {
            restore_with_input(state, session, notifier, context, backend, heads, bindings)
        }
        Err(error) => restore_without_input(state, session, notifier, heads, bindings, error),
    }
}

/// The restore tail when the fresh input built: swaps session and input
/// onto the new seat client and puts `heads` back -- possibly empty (see
/// `restore`'s doc for the drives-nothing survival shape).
fn restore_with_input(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    context: Libinput,
    backend: LibinputInputBackend,
    heads: Vec<super::head::Head>,
    bindings: &[(connector::Handle, OutputId)],
) {
    let empty = heads.is_empty();
    let old_libinput_token = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.libinput_token
    };
    state.loop_handle.remove(old_libinput_token);
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register the session during shutdown"
            );
            return;
        }
    };
    let libinput_token = match state
        .loop_handle
        .insert_source(backend, super::libinput_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register input during shutdown"
            );
            return;
        }
    };
    // Heads and wiring back in place. The display stays dark: `active`
    // was already false from the failed reactivation that armed this
    // attempt, and it is asserted again here rather than trusted -- a
    // restored backend reports `live: false` until a later switch back
    // proves master, never on the strength of the old fd.
    {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.session = session;
        tty.libinput = context;
        tty.session_token = session_token;
        tty.libinput_token = libinput_token;
        // An empty restore holds the primary the way a fully-unplugged
        // hotplug does (see `hotplug.rs`'s `Hold`): the next connector
        // change retries rather than planning `Keep` on nothing. Only an
        // actually empty probe holds: a master-less rebuild left
        // connected displays behind, and claiming a hold would throw away
        // the retry the next identical probe owes.
        tty.nothing_connected = empty;
        tty.active = false;
        tty.heads = heads;
    }
    rebind(state, bindings);
    if empty {
        tracing::error!(
            "seat reconnect did not recover the display: {}; the session stays alive \
             (keyboard and `scoot msg` answer, every output reports live:false) -- switch \
             VTs away and back to retry once a display is connected, or restart the session",
            empty_device_guidance()
        );
        return;
    }
    tracing::error!(
        "seat reconnect did not recover the display: {}; the session stays alive \
         (keyboard and `scoot msg` answer, every output reports live:false) -- switch \
         VTs away and back to retry, or restart the session",
        restore_guidance()
    );
}

/// The restore tail when the fresh input refused to build: swaps the
/// session onto the new seat client but keeps the old input wiring
/// untouched, then puts `heads` back. The keyboard may be dead with the
/// old context, but IPC answers and the next switch back retries -- never
/// a stranded session, never an exit.
fn restore_without_input(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    heads: Vec<super::head::Head>,
    bindings: &[(connector::Handle, OutputId)],
    input_error: String,
) {
    let empty = heads.is_empty();
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register the session during shutdown"
            );
            return;
        }
    };
    {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.session = session;
        tty.session_token = session_token;
        tty.nothing_connected = empty;
        tty.active = false;
        tty.heads = heads;
    }
    rebind(state, bindings);
    tracing::error!(
        %input_error,
        "seat reconnect did not recover the display ({}); the session stays alive \
         (`scoot msg` answers, every output reports live:false) -- switch VTs away and \
         back to retry, or restart the session",
        restore_guidance()
    );
}

/// Rebuilds GPU-tier heads on the *old* device after the fresh build (or
/// the fresh input) failed, and rewires the session and input on the new
/// seat client: today's `live: false` with a working keyboard, working
/// `change_vt`, and future retries -- never a stranded session and never a
/// process exit. Only compiled with the GPU tier present.
///
/// Mirrors [`restore`] one step at a time, with the renderer handoff added:
/// the old device is re-probed fresh (connectors may have moved while the
/// session was away), heads are rebuilt on the session's own tier through
/// `build_heads` (a head that cannot rejoin the GPU tier is refused rather
/// than mixed in, exactly as at startup -- see `hotplug.rs`'s per-head
/// plan), and each rebuilt renderer joins its output's backend through
/// `Backend::replace_scanout`. An unreadable device probes as
/// nothing-connected; an empty rebuild keeps the session headless-dark
/// with the same survival shape [`restore`] documents.
///
/// The `state.tty is None` guards below are unreachable single-threaded
/// (see [`restore`]'s doc) -- each is a `debug_assert` plus a loud early
/// return, never an exit. Loop-insert refusals can only fire while the loop
/// is shutting down: they log and return so a clean teardown is not
/// misreported as a crash.
#[cfg(feature = "gpu-scanout")]
fn restore_scanout(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    bindings: &[(connector::Handle, OutputId)],
    modes: &ModeRequests,
    wanted: RendererKind,
) {
    // Fresh probe, not the startup cache: the connectors may have moved
    // while the session was away, and the restore must see what is there
    // now (same freshness the hotplug path probes with). An unreadable
    // device probes as nothing-connected rather than exiting: the empty
    // survival below keeps the session alive for the retry.
    let connected = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        match tty.drm.resource_handles() {
            Ok(resources) => {
                super::gpu::find_all(&tty.drm, &resources, modes, super::gpu::Freshness::Reprobe)
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    "seat reconnect restore: cannot re-read the old device; keeping the session dark \
                     until a later switch back retries it"
                );
                Vec::new()
            }
        }
    };
    // The session's tier is settled (it drove these outputs minutes ago);
    // keep it rather than re-deciding: a head that cannot rejoin its own
    // tier is refused, exactly as at startup. Whether the probe saw
    // anything connected rides along separately: an empty rebuild with
    // displays still connected is the master-less shape (a GPU compositor
    // needs master even to test its modeset, and the old fd has none --
    // that is why this reconnect is running at all), not an unplug, and
    // its message must say so.
    let anything_connected = !connected.is_empty();
    let (built, _decided, _failures) = {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        let drm_fd = tty.drm.device_fd().clone();
        super::build_heads(&mut tty.drm, &drm_fd, connected, Some(wanted), wanted)
    };
    let mut heads: Vec<super::head::Head> = Vec::with_capacity(built.len());
    let mut handoffs: Vec<(connector::Handle, crate::compositor::render::ScanoutHandoff)> =
        Vec::with_capacity(built.len());
    for (head, handoff) in built {
        handoffs.push((head.connector, handoff));
        heads.push(head);
    }
    // Rewire input on the new session first: the old context's interface
    // holds the dropped (dead) session, so it cannot survive the swap.
    // The old input source is removed only once the replacement is built.
    // A rebuild failure keeps the old input wiring rather than exiting:
    // the old context may be dead with it, but the session stays alive
    // for IPC and the next switch back retries -- strictly better than a
    // kill.
    match rebuild_input(&session) {
        Ok((context, backend)) => {
            restore_scanout_with_input(
                state,
                session,
                notifier,
                context,
                backend,
                heads,
                handoffs,
                bindings,
                anything_connected,
            );
        }
        Err(error) => {
            restore_scanout_without_input(
                state,
                session,
                notifier,
                heads,
                handoffs,
                bindings,
                anything_connected,
                error,
            );
        }
    }
}

/// The GPU-tier restore tail when the fresh input built: swaps session and
/// input onto the new seat client, puts `heads` back -- possibly empty (see
/// [`restore`]'s doc for the drives-nothing survival shape) -- and hands
/// each rebuilt renderer to its output's backend. Only compiled with the
/// GPU tier present.
#[cfg(feature = "gpu-scanout")]
#[allow(clippy::too_many_arguments)]
fn restore_scanout_with_input(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    context: Libinput,
    backend: LibinputInputBackend,
    heads: Vec<super::head::Head>,
    mut handoffs: Vec<(connector::Handle, crate::compositor::render::ScanoutHandoff)>,
    bindings: &[(connector::Handle, OutputId)],
    anything_connected: bool,
) {
    let empty = heads.is_empty();
    // An empty rebuild with displays still connected is the master-less
    // shape, not an unplug (see `restore_scanout`): its message must name
    // the missing master and the seat-level retry, not a missing display.
    // A genuinely empty probe keeps the unplug wording and the hold.
    let masterless_empty = empty && anything_connected;
    let old_libinput_token = {
        let Some(tty) = state.tty.as_ref() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.libinput_token
    };
    state.loop_handle.remove(old_libinput_token);
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register the session during shutdown"
            );
            return;
        }
    };
    let libinput_token = match state
        .loop_handle
        .insert_source(backend, super::libinput_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register input during shutdown"
            );
            return;
        }
    };
    // Heads and wiring back in place. The display stays dark: `active`
    // was already false from the failed reactivation that armed this
    // attempt, and it is asserted again here rather than trusted -- a
    // restored backend reports `live: false` until a later switch back
    // proves master, never on the strength of the old fd.
    {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.session = session;
        tty.libinput = context;
        tty.session_token = session_token;
        tty.libinput_token = libinput_token;
        // An empty restore holds the primary the way a fully-unplugged
        // hotplug does (see `hotplug.rs`'s `Hold`): the next connector
        // change retries rather than planning `Keep` on nothing. Only an
        // actually empty probe holds: a master-less rebuild left
        // connected displays behind, and claiming a hold would throw away
        // the retry the next identical probe owes.
        tty.nothing_connected = empty && !anything_connected;
        tty.active = false;
        tty.heads = heads;
    }
    rebind(state, bindings);
    // Each rebuilt renderer joins its output's backend, by connector. A
    // head the rebind dropped leaves its renderer to drop with it; a
    // backend without one keeps its old renderer, which then has no
    // presenter and draws nothing until the next switch back -- dark, but
    // never mismatched mid-frame, and never a kill.
    for (connector, id) in bindings {
        let position = handoffs
            .iter()
            .position(|(head_connector, _)| *head_connector == *connector);
        let Some(position) = position else {
            continue;
        };
        let (_, handoff) = handoffs.remove(position);
        let Some(handoff) = handoff.backend else {
            continue;
        };
        if !state
            .backends
            .get_mut(id)
            .is_some_and(|backend| backend.replace_scanout(handoff))
        {
            tracing::error!(
                output = id.0,
                "seat reconnect restore: no GPU-tier backend to hand the rebuilt renderer to"
            );
        }
    }
    if empty {
        // An empty rebuild with displays still connected is the
        // master-less shape, not an unplug: name the missing master and
        // the seat-level retry rather than a missing display.
        let guidance = if masterless_empty {
            masterless_rebuild_guidance()
        } else {
            empty_device_guidance()
        };
        tracing::error!(
            "seat reconnect did not recover the display: {}; the session stays alive \
             (keyboard and `scoot msg` answer, every output reports live:false) -- switch \
             VTs away and back to retry once a display is connected, or restart the session",
            guidance
        );
        return;
    }
    tracing::error!(
        "seat reconnect did not recover the display: {}; the session stays alive \
         (keyboard and `scoot msg` answer, every output reports live:false) -- switch \
         VTs away and back to retry, or restart the session",
        restore_guidance()
    );
}

/// The GPU-tier restore tail when the fresh input refused to build: swaps
/// the session onto the new seat client but keeps the old input wiring
/// untouched, then puts `heads` back and hands each rebuilt renderer to
/// its output's backend. The keyboard may be dead with the old context,
/// but IPC answers and the next switch back retries -- never a stranded
/// session, never an exit. Only compiled with the GPU tier present.
#[cfg(feature = "gpu-scanout")]
#[allow(clippy::too_many_arguments)]
fn restore_scanout_without_input(
    state: &mut State,
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    heads: Vec<super::head::Head>,
    mut handoffs: Vec<(connector::Handle, crate::compositor::render::ScanoutHandoff)>,
    bindings: &[(connector::Handle, OutputId)],
    anything_connected: bool,
    input_error: String,
) {
    let empty = heads.is_empty();
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore: cannot re-register the session during shutdown"
            );
            return;
        }
    };
    {
        let Some(tty) = state.tty.as_mut() else {
            debug_assert!(
                state.tty.is_some(),
                "tty vanished mid-restore with no dispatch between"
            );
            tracing::error!("seat reconnect restore: lost the old backend; keeping the session");
            return;
        };
        tty.session = session;
        tty.session_token = session_token;
        // Only an actually empty probe holds the primary (see
        // `restore_scanout_with_input`): a master-less rebuild left
        // connected displays behind.
        tty.nothing_connected = empty && !anything_connected;
        tty.active = false;
        tty.heads = heads;
    }
    rebind(state, bindings);
    for (connector, id) in bindings {
        let position = handoffs
            .iter()
            .position(|(head_connector, _)| *head_connector == *connector);
        let Some(position) = position else {
            continue;
        };
        let (_, handoff) = handoffs.remove(position);
        let Some(handoff) = handoff.backend else {
            continue;
        };
        if !state
            .backends
            .get_mut(id)
            .is_some_and(|backend| backend.replace_scanout(handoff))
        {
            tracing::error!(
                output = id.0,
                "seat reconnect restore: no GPU-tier backend to hand the rebuilt renderer to"
            );
        }
    }
    tracing::error!(
        %input_error,
        "seat reconnect did not recover the display ({}); the session stays alive \
         (`scoot msg` answers, every output reports live:false) -- switch VTs away and \
         back to retry, or restart the session",
        restore_guidance()
    );
}

/// Binds rebuilt heads to the outputs the compositor already has, by the
/// connector snapshot taken before teardown; drops heads no output claims
/// and removes outputs no head drives (mirroring the hotplug path's
/// remove/retain discipline, with reconnect-specific log lines).
fn rebind(state: &mut State, bindings: &[(connector::Handle, OutputId)]) {
    for (connector, id) in bindings {
        let connector = *connector;
        super::attach_where(
            state,
            move |head| head.connector == connector && head.output.is_none(),
            *id,
        );
    }
    super::retain_attached(state);
    remove_orphaned_outputs(state, bindings);
}

/// Outputs whose head is gone (their connector vanished mid-switch):
/// remove them the way a hotplug would, so no output points at a head
/// that no longer exists. Shared by [`rebind`] and the GPU-tier swap,
/// which binds the same way but must not drop the heads no binding
/// covers (see `swap_scanout`).
fn remove_orphaned_outputs(state: &mut State, bindings: &[(connector::Handle, OutputId)]) {
    // Outputs whose head is gone (their connector vanished mid-switch):
    // remove them the way a hotplug would, so no output points at a head
    // that no longer exists.
    let orphaned: Vec<OutputId> = bindings
        .iter()
        .map(|(_, id)| *id)
        .filter(|id| {
            state
                .tty
                .as_ref()
                .is_none_or(|tty| !tty.heads.iter().any(|head| head.presents(*id)))
        })
        .collect();
    for id in orphaned {
        tracing::info!(
            output = id.0,
            "seat reconnect: the display this output drove is gone; removing its output"
        );
        if !state.remove_output(id) {
            tracing::error!(
                output = id.0,
                "seat reconnect: could not remove the output of a display that went away"
            );
        }
    }
    state.reapply_output_power();
}

/// The actionable half of a reconnect that fell back to the old device:
/// what happened, in the log line's own words. A pure function so
/// headless tests pin its wording -- the fd half needs real seat
/// hardware, this half must not rot silently.
fn restore_guidance() -> String {
    "the seat reconnect did not produce a working device".to_owned()
}

/// The actionable half of a reconnect that opened a fresh device but could
/// not modeset on it: why the fresh fd stays dark. A pure function so
/// headless tests pin its wording apart from [`restore_guidance`]'s.
fn keep_dark_guidance() -> String {
    "the fresh device refused its first modeset, so it holds no proven master".to_owned()
}

/// The actionable half of a restore that found the old device driving
/// nothing: why the session stays headless-dark. A pure function so
/// headless tests pin its wording apart from [`restore_guidance`]'s -- the
/// fd half needs real seat hardware, this half must not rot silently.
fn empty_device_guidance() -> String {
    "the old device drives nothing now, so the session stays headless-dark (every output \
     reports live:false) until a display is connected and a later switch back retries it"
        .to_owned()
}

/// The actionable half of a GPU-tier restore whose rebuild came back
/// empty while displays are still connected: the old fd holds no master
/// (that is why the reconnect is running at all), and building a GPU
/// compositor needs one even to test its modeset -- so the heads cannot
/// be rebuilt on it, unlike CPU heads, which need no master to build.
/// Why the session stays headless-dark, and what retries it. A pure
/// function so headless tests pin its wording apart from
/// [`empty_device_guidance`]'s -- the fd half needs real seat hardware,
/// this half must not rot silently. Only the GPU-tier restore calls it
/// outside tests, so it is gated on either.
#[cfg(any(test, feature = "gpu-scanout"))]
fn masterless_rebuild_guidance() -> String {
    "the old device is still master-less, so its GPU heads cannot be rebuilt on it \
     (every output reports live:false) -- switch VTs away and back to retry, which \
     reconnects the seat anew on a healthy fd, or restart the session"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_vacant_shape_arms() {
        for outcome in [
            Reactivation::Recovered,
            Reactivation::LostHeld,
            Reactivation::LostVacant,
            Reactivation::Failed,
        ] {
            assert_eq!(
                should_attempt(outcome),
                matches!(outcome, Reactivation::LostVacant),
                "{outcome:?}"
            );
        }
    }

    #[test]
    fn the_restore_guidance_names_no_working_device() {
        let guidance = restore_guidance();
        assert!(
            guidance.contains("did not produce a working device"),
            "{guidance}"
        );
    }

    #[test]
    fn the_keep_dark_guidance_names_the_unproven_master() {
        let guidance = keep_dark_guidance();
        assert!(guidance.contains("no proven master"), "{guidance}");
    }

    #[test]
    fn only_a_vacant_idle_unpaused_session_proceeds() {
        // The fail-first pin for `run`'s gate: the vacant diagnosis on a
        // master-less, unpaused session is the only shape that proceeds.
        assert!(should_proceed(Reactivation::LostVacant, false, false));
    }

    #[test]
    fn a_superseded_diagnosis_never_proceeds() {
        // N2: a second activation before the idle runs overwrites the
        // snapshot (see `arm`), and the queued idle must stand down on
        // anything but the vacant shape -- acting on a stale vacant one
        // would tear down a backend the follow-up already settled.
        for diagnosis in [
            Reactivation::Recovered,
            Reactivation::LostHeld,
            Reactivation::Failed,
        ] {
            assert!(!should_proceed(diagnosis, false, false), "{diagnosis:?}");
        }
    }

    #[test]
    fn a_recovered_session_never_proceeds() {
        // The session won its own race between the arm and the idle: an
        // active session holds master, so there is nothing to reconnect.
        assert!(!should_proceed(Reactivation::LostVacant, true, false));
    }

    #[test]
    fn a_paused_session_never_proceeds() {
        // N1: switched away again before the idle ran -- rebuilding while
        // paused would open on an inactive client and wrongly clear
        // `session_paused` on the swap path. The next switch back arms its
        // own attempt.
        assert!(!should_proceed(Reactivation::LostVacant, false, true));
        assert!(!should_proceed(Reactivation::LostVacant, true, true));
    }

    #[test]
    fn the_masterless_rebuild_guidance_names_master_and_the_retry() {
        // The GPU-tier empty-with-connected shape is not an unplug: the
        // message must name the missing master (not a missing display)
        // and the switch-back retry. Drop any of those phrases and this
        // fails.
        let guidance = masterless_rebuild_guidance();
        assert!(guidance.contains("master-less"), "{guidance}");
        assert!(guidance.contains("live:false"), "{guidance}");
        assert!(guidance.contains("switch VTs away and back"), "{guidance}");
    }

    #[test]
    fn the_empty_device_guidance_names_the_dark_session_and_the_retry() {
        // N3: the drives-nothing survival keeps the wiring dark instead of
        // exiting -- the message must say what stayed alive, what reads
        // dead, and what retries it. Drop any of those phrases and this
        // fails.
        let guidance = empty_device_guidance();
        assert!(guidance.contains("drives nothing"), "{guidance}");
        assert!(guidance.contains("live:false"), "{guidance}");
        assert!(guidance.contains("switch back"), "{guidance}");
    }

    #[test]
    fn only_a_matching_tier_swaps() {
        // One tier per session: a fresh device that fell back to the other
        // tier restores the old device instead of mixing tiers (a CPU head
        // in a GPU session would be rebuilt as the offscreen pipeline the
        // tier choice exists to avoid). Same tier on both sides proceeds;
        // anything else restores.
        assert!(fresh_tier_matches(
            RendererKind::Pixman,
            RendererKind::Pixman
        ));
        assert!(fresh_tier_matches(RendererKind::Gles, RendererKind::Gles));
        assert!(!fresh_tier_matches(
            RendererKind::Pixman,
            RendererKind::Gles
        ));
        assert!(!fresh_tier_matches(
            RendererKind::Gles,
            RendererKind::Pixman
        ));
    }
}
