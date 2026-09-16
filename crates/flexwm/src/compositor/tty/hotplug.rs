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
//! `flexwm msg outputs`. The alternative -- destroying and recreating the
//! `wl_output` -- would make every client re-enter the output, re-map its
//! layer surfaces and re-read its scale, which is a far bigger lie about
//! what happened than a stale name.

#[cfg(test)]
mod tests;

use smithay::backend::drm::DrmSurface;
use smithay::backend::udev::{UdevDevices, UdevEvent};
use smithay::reexports::drm::control::{Device as ControlDevice, Mode, connector};

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
                     screen will stay as it is -- flexwm drives one DRM device, \
                     chosen at startup, and cannot move the session to another \
                     one. Restart flexwm once the device is back."
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
}

impl Reconfigured {
    /// Does whatever this outcome owes the rest of `State`. Called once,
    /// after the `&mut Tty` borrow that produced it has ended -- from
    /// [`udev_event`] and from `session_event`'s reactivation arm.
    pub(super) fn finish(self, state: &mut State) {
        match self {
            Self::Nothing => {}
            Self::Render => state.request_render(),
            Self::Resized(width, height) => {
                if !state.resize_output(width, height) {
                    // error!, not warn!: the DRM side has already been told
                    // to change mode, so the render target is now a
                    // different size from what `Tty::present` will accept
                    // and every frame from here on is dropped by its size
                    // guard. `resize_output` has logged what actually
                    // failed; this says what it costs.
                    tracing::error!(
                        width,
                        height,
                        "drm: the display changed mode but the render target \
                         could not follow; the screen stays as it is until the \
                         next hotplug or a restart"
                    );
                }
            }
        }
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
                         plug a display back in and flexwm mode-sets onto it."
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
                // Cleared on every probe that found *something*, including
                // an otherwise uninteresting one: this field means "the last
                // probe found nothing `Connected`", nothing more. It is not
                // about DRM master (`active`) or the session (`session_paused`).
                let reconnected = std::mem::replace(&mut self.nothing_connected, false);
                match plan {
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
                        self.invalidate_scanout();
                        Reconfigured::Render
                    }
                    Plan::Unchanged | Plan::NoConnector => {
                        tracing::debug!("drm: hotplug changed nothing this backend is driving");
                        Reconfigured::Nothing
                    }
                    Plan::NewMode | Plan::NewConnector => self.retarget(connector, mode, &name),
                }
            }
        }
    }

    /// Moves this backend onto `connector`/`mode`: new scanout buffers if
    /// the size changed, the surface pointed at the new state, and every
    /// piece of scanout bookkeeping invalidated so the next frame is a full
    /// modeset rather than a page flip onto a CRTC that is no longer
    /// showing what this pool thinks it is.
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
        // The previous pair is read from the surface, not from this struct's
        // own fields, because it is the surface's *pending* state that a
        // half-applied change has to be undone back to.
        let previous = (self.connector, self.surface.pending_mode());
        if !set_pending(&self.surface, (connector, mode), previous) {
            // `buffers` is dropped here, releasing the dumb buffers that
            // were allocated for a mode the surface refused.
            return Reconfigured::Nothing;
        }
        tracing::info!(
            connector = %name,
            width,
            height,
            "drm: display reconfigured; mode-setting onto it"
        );
        self.connector = connector;
        if let Some(buffers) = buffers {
            // The old pool -- and the framebuffers in it, one of which the
            // CRTC may still be scanning out -- is dropped here. The kernel
            // handles a framebuffer removed while active by blanking the
            // plane, which is what a mode change does anyway; the full
            // modeset `invalidate_scanout` arms below is what brings it
            // back, on a buffer that is the right size for the new mode.
            self.buffers = buffers;
        }
        // Unconditionally, not inside the branch above: these two are
        // `present`'s size guard and must equal the size the pool was built
        // at, whichever branch got here. Writing them only where a new pool
        // was built would make that a fact about this `if`, which is exactly
        // the kind of coupling that survives one refactor and not two. When
        // the size did not change they are already these values.
        self.width = width;
        self.height = height;
        self.invalidate_scanout();
        if size_changed {
            Reconfigured::Resized(width, height)
        } else {
            Reconfigured::Render
        }
    }

    /// The scanout bookkeeping shared by `reactivate` and [`Self::retarget`]:
    /// after either, nothing this pool records can be trusted to describe
    /// what the CRTC is showing, and only a full modeset -- not a page flip
    /// onto state that may have been reconfigured behind us -- is safe to
    /// issue next.
    ///
    /// `flip_pending` is cleared even though a flip really may still be in
    /// flight. Both directions have a cost and they are not symmetric:
    /// leaving it set when the flip's `VBlank` never arrives (its
    /// framebuffer having been destroyed, or its CRTC re-modeset underneath
    /// it) freezes the screen permanently with no error anywhere, while
    /// clearing it costs at worst one rejected flip, which `present` logs
    /// and retries from the next `VBlank` (see its error arm).
    pub(super) fn invalidate_scanout(&mut self) {
        self.flip_pending = false;
        self.needs_modeset = true;
        self.buffers.mark_all_free();
        self.buffers.invalidate_ages();
        self.showing = None;
        self.pending_free = None;
    }
}

/// Points `surface` at `target`'s connector and mode as the state for its
/// next commit, leaving the surface's pending state as it found it if it
/// cannot. Returns whether both took.
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
    match surface.set_connectors(&[connector]) {
        Ok(()) => match surface.use_mode(mode) {
            Ok(()) => return true,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "drm: new connector accepted but not with the new mode; \
                     trying the other order"
                );
                if let Err(error) = surface.set_connectors(&[previous_connector]) {
                    // Not fatal, and not even unhelpful: the second attempt
                    // below sets the mode first and then this same connector,
                    // so a failed undo leaves the surface closer to the
                    // target, not further from it.
                    tracing::debug!(%error, "drm: could not put the previous connector back");
                }
            }
        },
        Err(error) => {
            tracing::debug!(
                %error,
                "drm: new connector rejected with the current mode; trying the \
                 other order"
            );
        }
    }
    match surface.use_mode(mode) {
        Ok(()) => match surface.set_connectors(&[connector]) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "drm: could not move the surface onto the new connector in \
                     either order; staying on the current one"
                );
                if let Err(error) = surface.use_mode(previous_mode) {
                    tracing::warn!(%error, "drm: could not put the previous mode back either");
                }
                false
            }
        },
        Err(error) => {
            tracing::warn!(
                %error,
                "drm: could not move the surface onto the new connector in \
                 either order; staying on the current one"
            );
            false
        }
    }
}

/// A mode's size in the `i32` physical pixels every other size in this
/// compositor is counted in. A DRM mode's own `u16` pair cannot overflow
/// the conversion, which is why this is a widening `from` and not a
/// fallible one.
fn mode_size(mode: Mode) -> (i32, i32) {
    let (width, height) = mode.size();
    (i32::from(width), i32::from(height))
}
