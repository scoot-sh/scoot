//! Following the display underneath `--tty`: DRM hotplug, and the host
//! display reconfiguration that looks exactly like it.
//!
//! `tty/mod.rs` picks a connector and a mode once, at startup, and
//! `gpu.rs` decides which. This module is what happens when the answer
//! stops being true while the session is running. Two shapes of the same
//! event (issue #48):
//!
//! - **A monitor is plugged in or unplugged.** Unplugging the connector
//!   `--tty` was driving used to leave a black screen until restart, because
//!   nothing re-ran the choice.
//! - **The host reconfigures the VM's display.** Apple's Virtualization
//!   framework (vfkit, UTM) sets `automaticallyReconfiguresDisplay`, so
//!   moving the window between a 1x and a 2x screen, resizing it or going
//!   full-screen hands virtio-gpu a new host size; the guest kernel drops a
//!   hotplug event and re-probes the connector with a different mode list
//!   and a different preferred mode. `--mode WxH` exists as the workaround
//!   for this and only works while the pinned size stays in the list.
//!
//! The kernel reports both the same way -- a `change` uevent on the DRM
//! card -- which is what [`smithay::backend::udev::UdevBackend`] turns into
//! [`UdevEvent::Changed`]. `tty/mod.rs`'s `init` registers one with the
//! event loop; [`udev_event`] below is its handler.
//!
//! # One output, still
//!
//! Multi-output remains out of scope for this backend, and this module does
//! not smuggle it in: a hotplug re-runs the *same* single-connector choice
//! startup made ([`gpu::reselect`]), it does not start driving a second
//! screen. Concretely, plugging a monitor into a laptop already running on
//! `eDP-1` keeps the session on `eDP-1` -- see `gpu::reselect`'s doc for why
//! staying put is the only defensible answer when one of the two connectors
//! has to be dark.
//!
//! One thing that does *not* follow the connector: the `wl_output`'s name.
//! It is fixed when the output is created (`headless::init_named`, from the
//! connector `tty::init` chose) and Smithay's `Output` has no way to rename
//! one. So a session that started on `HDMI-A-1` and fell back to `eDP-1`
//! when the HDMI cable came out keeps reporting `HDMI-A-1` to clients and to
//! `scoot msg outputs`. The alternative -- destroying and recreating the
//! `wl_output` -- would make every client re-enter the output, re-map its
//! layer surfaces and re-read its scale, which is a far bigger lie about
//! what happened than a stale name.

#[cfg(test)]
mod tests;

use smithay::backend::drm::DrmSurface;
use smithay::backend::udev::{UdevDevices, UdevEvent};
use smithay::reexports::drm::control::{Device as ControlDevice, Mode, connector, crtc};

use super::buffers::BufferPool;
use super::{State, Tty, gpu};

/// Handles one udev event for the DRM subsystem.
///
/// Only events for the device this backend is actually driving mean
/// anything here -- a seat with two GPUs on it (a laptop's integrated
/// display controller and an external dock, say) reports changes on both,
/// and only one of them is ours. `--tty` drives exactly one device, chosen
/// once at startup; adopting a different one mid-session is multi-GPU
/// support, which this backend does not have.
pub(super) fn udev_event(event: UdevEvent, _: &mut UdevDevices, state: &mut State) {
    // Scoped so the mutable borrow of `state.tty` ends before the calls
    // below need `state` whole again -- same shape as `session_event`.
    let outcome = {
        let Some(tty) = &mut state.tty else {
            return;
        };
        match event {
            UdevEvent::Changed { device_id } if device_id == tty.device_id => tty.reconfigure(),
            UdevEvent::Removed { device_id } if device_id == tty.device_id => {
                // error!, not warn!: the device this session's display lives
                // on is gone, every subsequent flip will fail, and there is
                // no recovery path in this backend -- adopting another device
                // is multi-GPU support. Saying so once is the difference
                // between a diagnosable log and a stream of "drm commit
                // failed" warnings with no cause attached.
                tracing::error!(
                    "drm: the device this session is driving was removed; the \
                     screen will stay as it is -- scoot drives one DRM device, \
                     chosen at startup, and cannot move the session to another \
                     one. Restart scoot once the device is back."
                );
                Reconfigured::Nothing
            }
            // Every other device's events, and `Added` for any device: a new
            // GPU appearing on the seat is not something a single-device
            // backend can use.
            _ => Reconfigured::Nothing,
        }
    };
    outcome.finish(state);
}

/// What [`Tty::reconfigure`] did, and so what its caller still owes the
/// rest of `State` once the borrow of `state.tty` has ended.
///
/// Split from the work itself because everything below needs `&mut Tty`
/// (which borrows `state`) while [`State::resize_output`] needs `&mut
/// State` whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reconfigured {
    /// Nothing this backend drives changed; no frame is owed.
    Nothing,
    /// The screen must be redrawn -- the scanout state was invalidated --
    /// but the output's size is what it already was, so nothing outside
    /// this backend needs to hear about it.
    Render,
    /// The output is now this many physical pixels wide and high, and the
    /// render target, the `wl_output`, layer surfaces and the core's layout
    /// all have to follow.
    Resized(i32, i32),
    /// The session moved to a different CRTC to follow the display (see
    /// [`Tty::switch_crtc`](super::Tty::switch_crtc)): `size_changed` is
    /// whether the mode size moved with it (the `Resized`-vs-`Render`
    /// question), and `gamma_size` is the new CRTC's LUT length for
    /// `zwlr_gamma_control_v1` (see `GammaControlState::crtc_changed`).
    SwitchedCrtc {
        width: i32,
        height: i32,
        size_changed: bool,
        gamma_size: u32,
    },
}

impl Reconfigured {
    /// Does whatever this outcome owes the rest of `State`. Called once,
    /// after the `&mut Tty` borrow that produced it has ended -- from
    /// [`udev_event`] and from `session_event`'s reactivation arm.
    pub(super) fn finish(self, state: &mut State) {
        match self {
            Self::Nothing => {}
            Self::Render => state.request_render(),
            Self::Resized(width, height) => apply_resize(state, width, height),
            Self::SwitchedCrtc {
                width,
                height,
                size_changed,
                gamma_size,
            } => {
                state.gamma_control.crtc_changed(gamma_size);
                if size_changed {
                    apply_resize(state, width, height);
                } else {
                    state.request_render();
                }
            }
        }
    }
}

/// Moves the render target and everything downstream of it onto a new
/// mode size: the shared body of `Reconfigured::{Resized, SwitchedCrtc}`'s
/// size-changing arms.
fn apply_resize(state: &mut State, width: i32, height: i32) {
    if !state.resize_output(width, height) {
        // error!, not warn!: the DRM side has already been told
        // to change mode, so the render target is now a
        // different size from what `Tty::present` will accept
        // and every frame from here on is dropped by its size
        // guard. `resize_output` has logged what actually
        // failed; this says what it costs.
        //
        // The wording is careful not to promise a retry that
        // cannot happen: `Tty::width`/`height` were updated
        // before this call, so as far as the next probe is
        // concerned this backend is *already* on the new mode.
        // Plugging the same display back in re-probes to the same
        // size, plans `Unchanged`, and never reaches here again.
        // Only a move to a genuinely different mode retries.
        tracing::error!(
            width,
            height,
            "drm: the display changed mode but the render target \
             could not follow, so every frame from here is dropped \
             as the wrong size. Only a change to a *different* mode \
             retries this -- the same display coming back at this \
             same size will not -- so a restart is the reliable way out"
        );
    }
}

/// What a fresh probe of the device means for what this backend is
/// currently driving.
///
/// Pure, and separated from the work so the decision itself is testable
/// without a DRM device: `probed` is what [`gpu::reselect`] found (`None`
/// when nothing on the device is `Connected` any more), `current` is the
/// connector and physical size this backend is on right now.
///
/// Sizes, not [`Mode`]s, deliberately. A re-probe hands back freshly
/// allocated `Mode`s whose raw `drm_mode_modeinfo` can differ from the ones
/// read at startup in fields that change nothing about scanout -- the
/// `PREFERRED` bit in `type` moves between modes as the kernel re-ranks a
/// new list, and `Mode`'s `PartialEq` compares the raw struct byte for
/// byte. Comparing modes with `==` would therefore report a change for a
/// re-probe that produced the same picture, and every such report costs a
/// real modeset, which visibly blanks the screen. The flip side is that a
/// mode change that keeps the size and only alters the timing (a different
/// refresh rate at the same resolution) is deliberately ignored: nothing
/// downstream would notice it either, since `headless.rs`'s `set_mode`
/// hard-codes 60 Hz on the `wl_output` regardless (see
/// `output_management.rs`).
fn plan(
    current: (connector::Handle, (i32, i32)),
    probed: Option<(connector::Handle, (i32, i32))>,
) -> Plan {
    let Some((connector, size)) = probed else {
        return Plan::NoConnector;
    };
    if connector != current.0 {
        Plan::NewConnector
    } else if size != current.1 {
        Plan::NewMode
    } else {
        Plan::Unchanged
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// The same connector at the same size -- the common case by far, since
    /// a `change` uevent fires for anything the kernel considers a change to
    /// the card, most of which this backend does not care about.
    Unchanged,
    /// The same connector, a different size: the vfkit/host-resize case, and
    /// a monitor that re-probed to a different preferred mode.
    NewMode,
    /// A different connector: the one being driven went away and something
    /// else is connected.
    NewConnector,
    /// Nothing on this device is `Connected` at all.
    NoConnector,
}

impl Tty {
    /// Re-runs the connector/mode choice against what the device says
    /// *now*, and applies the result if it differs from what is being
    /// driven.
    ///
    /// Called from [`udev_event`] on a `change` uevent for this device, and
    /// from `session_event`'s `ActivateSession` arm after a successful
    /// reactivation -- a monitor plugged in while the session was
    /// VT-switched away produces its uevent then, when this backend has no
    /// DRM master and cannot act on it, so the switch back has to ask again
    /// rather than assume nothing moved.
    pub(super) fn reconfigure(&mut self) -> Reconfigured {
        // Gated on `active` (DRM master held), not `session_paused`: every
        // step below past the probe is a modeset, and a modeset needs
        // master. These two are not the same question -- see `active`'s own
        // doc -- and the one that matters here is specifically "is the DRM
        // device ours to reconfigure", which is what `active` means. Nothing
        // is lost by skipping: `session_event` re-runs this after the next
        // successful `reactivate()`.
        if !self.active {
            tracing::debug!(
                "drm: ignoring a hotplug while this session does not hold drm \
                 master; it is re-read on the next reactivation"
            );
            return Reconfigured::Nothing;
        }
        // Read fresh, not cached from startup: the whole point is that the
        // set of connectors and their modes has changed underneath us.
        let resources = match self.drm.resource_handles() {
            Ok(resources) => resources,
            Err(error) => {
                tracing::warn!(%error, "drm: could not re-read the device's resources after a hotplug");
                return Reconfigured::Nothing;
            }
        };
        let found = gpu::reselect(&self.drm, &resources, self.connector, self.requested_mode);
        let current = (self.connector, (self.width, self.height));
        let probed = found
            .as_ref()
            .map(|&(connector, mode, _)| (connector, mode_size(mode)));
        match plan(current, probed) {
            Plan::NoConnector => {
                // Once per disconnection, not once per uevent: a display
                // being unplugged usually produces several `change` events
                // in a row, and this is a state the session can sit in for
                // hours.
                if !std::mem::replace(&mut self.nothing_connected, true) {
                    // warn!, not error!: nothing is broken and nothing is
                    // lost -- there is simply no display to drive. Plugging
                    // one back in recovers on its own, which is exactly what
                    // the message has to say, because the old behaviour here
                    // (a black screen until restart) taught the opposite.
                    tracing::warn!(
                        "drm: nothing is connected to this device any more; \
                         holding the last frame. The session keeps running -- \
                         plug a display back in and scoot mode-sets onto it."
                    );
                }
                Reconfigured::Nothing
            }
            plan => {
                // `plan` only returns the three below for a probe that found
                // something (see its doc). Restated as a `let else` rather
                // than an `expect`, because a panic here would take every
                // client's unsaved state with it to report a condition that
                // costs nothing to ignore.
                let Some((connector, mode, name)) = found else {
                    return Reconfigured::Nothing;
                };
                // Read, not taken. Clearing this is what records "a display
                // is back *and this backend has acted on it*", so it must not
                // happen before the acting: `retarget` below can still fail
                // (a refused buffer allocation, a surface that would not take
                // the new state), and clearing it first would throw away the
                // one thing that makes the next identical uevent retry rather
                // than plan `Unchanged` and do nothing. That is the same
                // two-sites-disagreeing-about-one-fact shape this module is
                // otherwise careful about.
                let reconnected = self.nothing_connected;
                let outcome = match plan {
                    Plan::Unchanged if reconnected => {
                        // The undock/redock case: the same monitor came back
                        // at the same mode, so nothing about the *choice*
                        // changed -- but the CRTC was driving a connector
                        // that physically went away in between, and whether
                        // it lights back up without a modeset is the
                        // driver's business, not something to bet a black
                        // screen on. Force one.
                        tracing::info!(
                            connector = %name,
                            "drm: a display is connected again; forcing a modeset"
                        );
                        self.presenter.invalidate_scanout();
                        Reconfigured::Render
                    }
                    Plan::Unchanged | Plan::NoConnector => {
                        tracing::debug!("drm: hotplug changed nothing this backend is driving");
                        Reconfigured::Nothing
                    }
                    Plan::NewMode | Plan::NewConnector => self.retarget(connector, mode, &name),
                };
                // Now, and only for an outcome that actually did something.
                // `Nothing` from the arms above means either that there was
                // never anything to act on (an uninteresting uevent, where
                // this is already `false`) or that acting on it failed, and
                // in the second case the next uevent has to find this still
                // set or it will not try again.
                if outcome != Reconfigured::Nothing {
                    self.nothing_connected = false;
                }
                outcome
            }
        }
    }

    /// Moves this backend onto `connector`/`mode`: new scanout buffers if
    /// the size changed, the surface pointed at the new state, and every
    /// piece of scanout bookkeeping invalidated so the next frame is a full
    /// modeset rather than a page flip onto a CRTC that is no longer
    /// showing what this pool thinks it is.
    ///
    /// When the current CRTC cannot take the new connector at all --
    /// encoders wired to specific CRTCs -- the in-place move is refused and
    /// this falls back to rebuilding the surface on a different CRTC (see
    /// `switch_crtc`) rather than staying on the connector that went away.
    ///
    /// Order is load-bearing. The buffers are allocated *first*, before
    /// anything on the DRM side is told to change: a failed allocation then
    /// leaves the display exactly as it was and working, whereas the other
    /// order would leave the CRTC on a mode whose frames no buffer in the
    /// pool is the right size to hold -- which `present`'s size guard drops
    /// silently, forever.
    fn retarget(&mut self, connector: connector::Handle, mode: Mode, name: &str) -> Reconfigured {
        let (width, height) = mode_size(mode);
        let size_changed = (width, height) != (self.width, self.height);
        // `None` when only the connector changed: the existing pool is
        // already the right size, and reallocating it would throw away two
        // perfectly good dumb buffers to get two identical ones.
        let buffers = if size_changed {
            match BufferPool::new(self.drm.device_fd(), width, height) {
                Ok(buffers) => Some(buffers),
                Err(error) => {
                    tracing::warn!(
                        %error, width, height,
                        "drm: could not allocate scanout buffers for the new mode; \
                         staying on the current one"
                    );
                    return Reconfigured::Nothing;
                }
            }
        } else {
            None
        };
        // Only the mode half of this pair is read from the surface --
        // `presenter.surface().pending_mode()` -- because it is the surface's
        // *pending* mode that a half-applied change has to be undone back
        // to. The connector half is `self.connector`, not
        // `pending_connectors()`: this field is the one this
        // module treats as authoritative for "which connector are we on",
        // and it is what the failure path below restores toward.
        let previous = (self.connector, self.presenter.surface().pending_mode());
        if !set_pending(self.presenter.surface(), (connector, mode), previous) {
            // The current CRTC cannot drive the new connector with the new
            // mode -- on hardware whose encoders are wired to specific CRTCs
            // (see `switch_crtc`) that is a routability refusal, not a mode
            // problem. Try a different CRTC before giving up; `buffers` moves
            // along (installed on success, dropped on failure, exactly as here).
            return self.switch_crtc(connector, mode, name, buffers, size_changed);
        }
        tracing::info!(
            connector = %name,
            width,
            height,
            "drm: display reconfigured; mode-setting onto it"
        );
        self.connector = connector;
        // The old pool -- and the framebuffers in it, one of which the CRTC
        // may still be scanning out -- is dropped inside `adopt_buffers`; see
        // its doc for what the kernel does with a framebuffer removed while
        // active. `None` (the size did not change) keeps the current pool.
        self.presenter.adopt_buffers(buffers);
        // Unconditionally, not inside `adopt_buffers`: these two are
        // `present`'s size guard and must equal the size the pool was built
        // at, whichever branch got here. Writing them only where a new pool
        // was built would make that a fact about that `if`, which is exactly
        // the kind of coupling that survives one refactor and not two. When
        // the size did not change they are already these values.
        self.width = width;
        self.height = height;
        self.presenter.invalidate_scanout();
        if size_changed {
            Reconfigured::Resized(width, height)
        } else {
            Reconfigured::Render
        }
    }

    /// Rebuilds the DRM surface on a different CRTC that can drive
    /// `connector`/`mode`, for when `set_pending` proved the current CRTC
    /// cannot: a display controller whose encoders are wired to specific
    /// CRTCs -- common on ARM SoCs, the same family of hardware as the
    /// split-GPU case `--gpu` exists for -- refuses the move with
    /// `TestFailed` (atomic) or a silent non-apply (legacy), and staying on
    /// the connector that just went away is a black screen. Called only from
    /// `retarget`, and only after the in-place move failed; the common case
    /// never reaches here.
    ///
    /// The live surface stays in place while candidates are tried: every CRTC
    /// but the current one owns its own primary plane, and
    /// `DrmDevice::create_surface` at the pinned rev claims exactly that
    /// plane -- so building the replacement first and swapping on success
    /// means a failed switch never touches what is on screen: the live
    /// surface keeps driving the old connector on the old CRTC. The current
    /// CRTC is not retried: the `set_pending` failure that led here already
    /// proved it refuses this target, and a fresh surface there would refuse
    /// it the same way. That is also what makes the take-and-replace refactor
    /// unnecessary: `surface` is never `None`, and every site that reads it
    /// (`present`, `on_vblank`, `reactivate`, `gamma_size`) keeps reading a
    /// live one -- the old CRTC's until the swap, the new one's after.
    ///
    /// One side effect a failed candidate does have, stated rather than
    /// hidden: dropping it runs Smithay's surface `Drop`, which clears that
    /// *candidate* CRTC's state (disables its current connectors, resets the
    /// CRTC). In the common single-display case that CRTC is idle and the
    /// clear is a no-op commit; at worst it drops whatever the firmware or
    /// console was showing on a display scoot never drove. What it can never
    /// touch is the live path: the clear addresses the candidate's own CRTC
    /// and its current connectors, which are disjoint from the old CRTC and
    /// the old connector by construction (a connector reports exactly one
    /// `CRTC_ID`). Each failed candidate also costs its probe's `TEST_ONLY`
    /// commits (atomic) -- all on the cable-move path, never per-frame.
    ///
    /// A candidate is validated before the swap, never trusted from
    /// construction: neither Smithay implementation checks routability in its
    /// constructor (atomic records the pending state blindly; legacy assigns
    /// it), so each candidate is probed with the same two setters
    /// `set_pending` uses -- connectors first, then the mode. The target's own
    /// values still exercise both checks: the atomic path test-commits them,
    /// and the legacy path runs each connector through its encoder and
    /// `possible_crtcs` check (whose silent non-apply is what `move_connector`
    /// reads back). Only a surface that takes both replaces the live one.
    ///
    /// Honest limit of that probe, on legacy only: a fresh surface's pending
    /// set already names the target (the constructor put it there), so the
    /// readback cannot catch the silent non-apply the way it does for the
    /// live surface -- a legacy candidate that fails its encoder check still
    /// probes green. The refusal then surfaces at the first real commit,
    /// through `present`'s existing failure arm (a warning, a bounded timer
    /// retry, then quiet until new damage), which ties the old behaviour:
    /// the previous connector is gone either way, so there is no working
    /// state the swap gives up. The atomic path -- the dev VM included --
    /// validates honestly, since its `TEST_ONLY` commit fails outright.
    ///
    /// `buffers` is the pool `retarget` allocated for the new size (`None`
    /// when only the connector changed): installed alongside the surface on
    /// success, dropped on failure exactly as `retarget`'s own failure path
    /// would. The gamma size is re-read from the new CRTC for the
    /// `SwitchedCrtc` outcome it returns -- per-CRTC hardware state (see
    /// `Tty::gamma_size`).
    ///
    /// Total failure degrades, never panics and never leaves the backend
    /// without a surface: it logs and returns `Nothing`, and `reconfigure`
    /// keeps `nothing_connected` set so the next uevent retries, exactly like
    /// a refused `set_pending`.
    fn switch_crtc(
        &mut self,
        connector: connector::Handle,
        mode: Mode,
        name: &str,
        buffers: Option<BufferPool>,
        size_changed: bool,
    ) -> Reconfigured {
        let (width, height) = mode_size(mode);
        let current = self.presenter.crtc();
        // Copied, not borrowed: `create_surface` below needs `&mut self.drm`.
        // One small `Vec` on a path that runs when a cable moves, matching
        // `tty::create_surface`'s own shape.
        let crtcs: Vec<crtc::Handle> = self
            .drm
            .crtcs()
            .iter()
            .copied()
            .filter(|&crtc| crtc != current)
            .collect();
        for crtc in crtcs {
            let candidate = match self.drm.create_surface(crtc, mode, &[connector]) {
                Ok(surface) => surface,
                Err(error) => {
                    tracing::debug!(?crtc, %error, "drm: a different crtc cannot take a surface for the new connector");
                    continue;
                }
            };
            if !move_connector(&candidate, connector) {
                tracing::debug!(
                    ?crtc,
                    "drm: a different crtc cannot drive the new connector"
                );
                continue;
            }
            if let Err(error) = candidate.use_mode(mode) {
                tracing::debug!(?crtc, %error, "drm: a different crtc cannot drive the new mode");
                continue;
            }
            tracing::info!(
                connector = %name,
                ?crtc,
                width,
                height,
                "drm: display reconfigured onto a different crtc; mode-setting onto it"
            );
            // The old surface -- and with it its primary-plane claim -- drops
            // here, once the replacement is proven. Its `Drop` clears the old
            // CRTC's state, whose connector is gone anyway; the full commit
            // `invalidate_scanout` arms below is what brings the new CRTC up
            // on the new state.
            self.presenter.adopt_surface(candidate);
            self.connector = connector;
            // The old pool -- and the framebuffers in it, one of which the
            // old CRTC may still be scanning out -- is dropped here, the
            // same blank-a-plane step `retarget` documents for the mode
            // change; the modeset below brings it back at the new size.
            self.presenter.adopt_buffers(buffers);
            // Unconditionally, for the same reason as in `retarget`: these two
            // are `present`'s size guard and must equal the size the pool was
            // built at, whichever branch got here.
            self.width = width;
            self.height = height;
            self.presenter.invalidate_scanout();
            // A new CRTC is new device state: the old connector's refusal
            // streak (if any) says nothing about the new one, so the first
            // transient refusal on it must arm a retry rather than answer
            // Quiet off a streak it never earned.
            self.presenter.reset_retries();
            return Reconfigured::SwitchedCrtc {
                width,
                height,
                size_changed,
                gamma_size: self.gamma_size(),
            };
        }
        tracing::warn!(
            connector = %name,
            "drm: no other crtc on this device can drive the new connector; staying on the current one"
        );
        Reconfigured::Nothing
    }
}

/// Points `surface` at `target`'s connector and mode as the state for its
/// next commit. Returns whether both took; on `false` it has tried to put
/// back whatever it managed to apply along the way, so the surface matches
/// what the caller still believes it is driving.
///
/// Both orderings are tried when the connector changes, because Smithay's
/// two setters each validate what is being set against the *other* one's
/// already-pending value: `set_connectors` test-commits the new connector
/// set with the mode currently pending, and `use_mode` test-commits the new
/// mode with the connectors currently pending (`backend/drm/surface/
/// atomic.rs` at the pinned rev builds exactly those two requests). When
/// both have to change at once -- the connector being driven was unplugged
/// and a different monitor, with its own mode list, took over -- whichever
/// goes first is tested against a value that is about to stop being true,
/// and either order can legitimately be rejected. The second attempt costs
/// one more `TEST_ONLY` atomic commit on a path that runs when a cable
/// moves, and is the difference between the display coming back and the
/// black-screen-until-restart this module exists to fix.
///
/// The restore is best-effort and can itself be refused, for the same
/// chicken-and-egg reason: putting the mode back is tested against the
/// connector set, and vice versa. That is logged rather than escalated,
/// because it is self-healing -- `Tty::connector` is left untouched, so the
/// next probe tries this connector again from a surface that is already
/// part-way there, and the pending state it disagrees with cannot reach the
/// CRTC before then (a `page_flip` commits plane state only; only the full
/// `commit` a modeset arms applies a pending mode or connector set).
fn set_pending(
    surface: &DrmSurface,
    target: (connector::Handle, Mode),
    previous: (connector::Handle, Mode),
) -> bool {
    let (connector, mode) = target;
    let (previous_connector, previous_mode) = previous;
    if connector == previous_connector {
        // Only the mode changed, so `use_mode` is tested against a connector
        // set that is already correct: one call, one order, nothing to undo.
        if let Err(error) = surface.use_mode(mode) {
            tracing::warn!(%error, "drm: the connector would not take the new mode");
            return false;
        }
        return true;
    }

    // Attempt 1: connectors, then mode.
    let mut connectors_moved = false;
    if move_connector(surface, connector) {
        connectors_moved = true;
        match surface.use_mode(mode) {
            Ok(()) => return true,
            Err(error) => tracing::debug!(
                %error,
                "drm: new connector accepted but not with the new mode; \
                 trying the other order"
            ),
        }
    }

    // Attempt 2: mode, then connectors. Attempt 1's connector move, if it
    // took, is deliberately *not* undone first -- it is half of what this
    // attempt is trying to reach, so undoing it would only make the mode
    // below harder to accept.
    let mut mode_moved = false;
    match surface.use_mode(mode) {
        Ok(()) => {
            mode_moved = true;
            if move_connector(surface, connector) {
                return true;
            }
            tracing::warn!(
                "drm: could not move the surface onto the new connector in \
                 either order; staying on the current one"
            );
        }
        Err(error) => tracing::warn!(
            %error,
            "drm: could not move the surface onto the new connector in either \
             order; staying on the current one"
        ),
    }

    // Neither order took. Put back only what was actually applied: a
    // `set_connectors`/`use_mode` that failed left `pending` untouched, so
    // re-asserting it would be a test commit for nothing -- and one that can
    // fail and log a warning about a state that was never wrong.
    if mode_moved {
        if let Err(error) = surface.use_mode(previous_mode) {
            tracing::warn!(%error, "drm: could not put the previous mode back either");
        }
    }
    // Through `move_connector`, not a raw `set_connectors`, for the same
    // reason the two attempts above are: on Smithay's legacy (non-atomic)
    // surface, `set_connectors` can answer `Ok` for a connector it did not
    // actually move to (see that function's doc) -- and the restore target
    // here is exactly the shape that triggers it, a connector that has just
    // disconnected and so offers an empty mode list. Without this, a legacy
    // restore failure would be silent: `pending_connectors()` would still
    // name whichever connector attempt 1 or 2 left it on, `Tty::connector`
    // would say `previous_connector`, and the two would disagree from here
    // on -- the exact fact-in-two-places split `move_connector` exists to
    // catch.
    if connectors_moved && !move_connector(surface, previous_connector) {
        tracing::warn!("drm: could not put the previous connector back either");
    }
    false
}

/// `DrmSurface::set_connectors(&[connector])`, plus a check that it
/// actually took. Returns whether the surface is now pending on
/// `connector`.
///
/// The check is not belt-and-braces. Smithay's **legacy** (non-atomic)
/// surface answers `Ok(())` for a connector its CRTC cannot drive and
/// simply leaves its pending set alone -- `surface/legacy.rs` at the pinned
/// rev collects every `check_connector` result and assigns the new set only
/// when they are all `true`, with no `else` and no error. (Its atomic
/// sibling does return `Err(TestFailed)`, so this only bites on legacy
/// hardware; the dev VM is atomic, so nothing there exercises it.) Taking
/// that `Ok` at face value would leave `Tty::connector` naming a connector
/// the surface is not driving -- one fact recorded in two places that
/// disagree, which is the failure mode this project has a whole roadmap
/// entry about (`docs/roadmap/05b-vt-switch-eperm.md`) and the one
/// `set_pending`'s caller relies on being true to pick a starting point for
/// the next probe.
///
/// Reading `pending_connectors()` back answers it for both implementations
/// without caring which is underneath, and costs one small `Vec` on a path
/// that has just done a `TEST_ONLY` atomic commit.
fn move_connector(surface: &DrmSurface, connector: connector::Handle) -> bool {
    if let Err(error) = surface.set_connectors(&[connector]) {
        tracing::debug!(
            %error,
            "drm: the surface refused the new connector with the mode it \
             currently has pending"
        );
        return false;
    }
    if surface
        .pending_connectors()
        .into_iter()
        .any(|pending| pending == connector)
    {
        return true;
    }
    tracing::debug!(
        "drm: the surface accepted the new connector without applying it, \
         which means its crtc cannot drive that connector with the mode \
         currently pending"
    );
    false
}

/// A mode's size in the `i32` physical pixels every other size in this
/// compositor is counted in. A DRM mode's own `u16` pair cannot overflow
/// the conversion, which is why this is a widening `from` and not a
/// fallible one.
fn mode_size(mode: Mode) -> (i32, i32) {
    let (width, height) = mode.size();
    (i32::from(width), i32::from(height))
}
