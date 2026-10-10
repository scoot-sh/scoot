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
//! the heads on the *old* device: today's `live: false` with a working
//! keyboard and future retries, never a stranded session. Only a dead seat
//! itself (the fresh connect failing, which dooms the old connection too)
//! exits, through the same supervisor-restarts-into-a-fresh-session
//! contract as any other seat loss.
//!
//! Deliberately dumb-tier-only: a scanout-tier rebuild would re-tie the
//! renderer's swapchain state this module does not touch, and the live
//! matrix runs the panel on dumb buffers. A scanout session logs once per
//! activation and stays on today's path.
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
/// runs `run` once dispatch settles. Coalesced by `reconnect_armed`, so a
/// flapping seat daemon arms one attempt, not one per event. Called with
/// the vacant diagnosis already established; re-checks nothing.
pub(super) fn arm(state: &mut State) {
    let Some(tty) = state.tty.as_mut() else {
        return;
    };
    if tty.reconnect_armed {
        return;
    }
    tty.reconnect_armed = true;
    tracing::info!(
        "seat reconnect armed: the seat daemon lost the master race on a vacant device; \
         trying one fresh seat client"
    );
    state.loop_handle.insert_idle(run);
}

/// One armed attempt. See the module doc for the shape; each numbered step
/// below names its fallback, so a reader can check that no failure leaves
/// the session worse than today's `live: false`.
fn run(state: &mut State) {
    let started = Instant::now();
    // The attempt is spent whatever happens below: clearing first means an
    // early return cannot leave a stale arm behind, and a second activation
    // arms its own attempt afterwards.
    let wanted = state.renderer;
    let Some(tty) = state.tty.as_mut() else {
        return;
    };
    tty.reconnect_armed = false;
    // The session may have recovered between the arm and this idle (a
    // second activation that won its own race): an active session holds
    // master, so there is nothing to reconnect. Tearing down a healthy
    // backend here would trade a live display for a needless swap.
    if tty.active {
        tracing::info!(
            "seat reconnect skipped: the session re-acquired drm master before the attempt ran"
        );
        return;
    }
    // Dumb tier only (see the module doc): a scanout rebuild
    // would re-tie renderer state this module does not touch. Nothing has
    // been torn down at this point, so returning keeps today's path whole.
    if tty.renderer() != RendererKind::Pixman {
        tracing::info!(
            "seat reconnect not attempted: this session is on the gpu scanout tier, \
             which the seat reconnect does not rebuild; keeping live:false until a \
             later switch back wins the race"
        );
        return;
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
    let device = match super::open_device(&mut session, &path, &modes, wanted) {
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
    } = device;
    let device_id = drm.device_id();

    // Read the old registrations and the carried-over state first, while
    // the old backend is still in place.
    let (old_drm_token, old_libinput_token, path, modes) = {
        let Some(tty) = state.tty.as_ref() else {
            tracing::error!("seat reconnect: lost the old backend mid-swap; stopping");
            std::process::exit(1);
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
    if state.tty.take().is_none() {
        tracing::error!("seat reconnect: lost the old backend mid-swap; stopping");
        std::process::exit(1);
    }
    // Wire the new sources. Failure here means the loop is shutting down
    // (nothing else refuses an insert); stopping matches that outcome.
    let session_token = match state
        .loop_handle
        .insert_source(notifier, super::session_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the session; stopping");
            std::process::exit(1);
        }
    };
    let drm_token = match state
        .loop_handle
        .insert_source(drm_notifier, super::drm_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register the drm device; stopping");
            std::process::exit(1);
        }
    };
    let libinput_token = match state
        .loop_handle
        .insert_source(backend, super::libinput_event)
    {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "seat reconnect: cannot re-register input; stopping");
            std::process::exit(1);
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
        reconnect_armed: false,
    });
    rebind(state, bindings);
    match live {
        SwapLive::Live => {
            // What the normal reactivation arm does after a win: ask the
            // device what it says now (a display may have moved while the
            // session was away), apply it, and render the modeset's frame.
            let outcomes = {
                let Some(tty) = state.tty.as_mut() else {
                    tracing::error!("seat reconnect: lost the new backend mid-swap; stopping");
                    std::process::exit(1);
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
/// `change_vt`, and future retries -- never a stranded session. Exits only
/// when the old device cannot drive a head either, which leaves nothing to
/// present on and nothing to retry with; a fresh start is the way back.
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
    // now (same freshness the hotplug path probes with).
    let connected = {
        let Some(tty) = state.tty.as_ref() else {
            tracing::error!("seat reconnect restore: lost the old backend; stopping");
            std::process::exit(1);
        };
        match tty.drm.resource_handles() {
            Ok(resources) => {
                super::gpu::find_all(&tty.drm, &resources, modes, super::gpu::Freshness::Reprobe)
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    "seat reconnect restore failed: cannot re-read the old device; stopping \
                     (a fresh start re-acquires master through the seat daemon)"
                );
                std::process::exit(1);
            }
        }
    };
    // The session's tier is settled (it drove these outputs minutes ago);
    // keep it rather than re-deciding: a head that cannot rejoin its own
    // tier is refused, exactly as at startup.
    let (heads, _decided, _failures) = {
        let Some(tty) = state.tty.as_mut() else {
            tracing::error!("seat reconnect restore: lost the old backend; stopping");
            std::process::exit(1);
        };
        let drm_fd = tty.drm.device_fd().clone();
        super::build_heads(&mut tty.drm, &drm_fd, connected, Some(wanted), wanted)
    };
    if heads.is_empty() {
        tracing::error!(
            "seat reconnect restore failed: the old device drives nothing now; stopping \
             (a fresh start re-acquires master through the seat daemon)"
        );
        std::process::exit(1);
    }
    // Rewire input on the new session first: the old context's interface
    // holds the dropped (dead) session, so it cannot survive the swap.
    // The old input source is removed only once the replacement is built.
    let (context, backend) = match rebuild_input(&session) {
        Ok(input) => input,
        Err(error) => {
            tracing::error!(
                %error,
                "seat reconnect restore failed: no input on the new session; stopping \
                 (a fresh start re-acquires master through the seat daemon)"
            );
            std::process::exit(1);
        }
    };
    let old_libinput_token = {
        let Some(tty) = state.tty.as_ref() else {
            tracing::error!("seat reconnect restore: lost the old backend; stopping");
            std::process::exit(1);
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
                "seat reconnect restore failed: cannot re-register the session; stopping"
            );
            std::process::exit(1);
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
                "seat reconnect restore failed: cannot re-register input; stopping"
            );
            std::process::exit(1);
        }
    };
    // Heads and wiring back in place. The display stays dark: `active`
    // was already false from the failed reactivation that armed this
    // attempt, and it is asserted again here rather than trusted -- a
    // restored backend reports `live: false` until a later switch back
    // proves master, never on the strength of the old fd.
    {
        let Some(tty) = state.tty.as_mut() else {
            tracing::error!("seat reconnect restore: lost the old backend; stopping");
            std::process::exit(1);
        };
        tty.session = session;
        tty.libinput = context;
        tty.session_token = session_token;
        tty.libinput_token = libinput_token;
        tty.nothing_connected = false;
        tty.active = false;
        tty.heads = heads.into_iter().map(|(head, _scanout)| head).collect();
    }
    rebind(state, bindings);
    tracing::error!(
        "seat reconnect did not recover the display: {}; the session stays alive \
         (keyboard and `scoot msg` answer, every output reports live:false) -- switch \
         VTs away and back to retry, or restart the session",
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
}
