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
//! # Every connected monitor, and never zero outputs
//!
//! Since milestone 19 phase E2 a hotplug is re-planned for every head at
//! once (`heads::replan`, which holds the rules): plugging a monitor into a
//! laptop already running on `eDP-1` *adds* an output for it and leaves the
//! panel's head exactly as it was; unplugging a monitor that is not the last
//! lit screen *removes* its output (`State::remove_output`), its windows and
//! workspaces handed to the output that adopts them. Only when nothing
//! driven is connected any more do the old single-output rules apply to the
//! primary head: it moves onto another connected connector if there is one
//! (issue #48's fallback) and otherwise holds its last frame. The last
//! output is never removed.
//!
//! One thing that does *not* follow the connector when the primary moves:
//! the `wl_output`'s name. It is fixed when the output is created and
//! Smithay's `Output` has no way to rename one. So a session whose only
//! screen was `HDMI-A-1` and fell back to `eDP-1` when the HDMI cable came
//! out keeps reporting `HDMI-A-1` to clients and to `scoot msg outputs`.
//! (With other screens still lit the head is removed instead, and an added
//! head's output is created under its own connector's name.) The alternative -- destroying and recreating the
//! `wl_output` -- would make every client re-enter the output, re-map its
//! layer surfaces and re-read its scale, which is a far bigger lie about
//! what happened than a stale name.

mod heads;
#[cfg(test)]
mod tests;

use scoot_core::OutputId;
use smithay::backend::drm::{DrmDevice, DrmSurface};
use smithay::backend::udev::{UdevDevices, UdevEvent};
use smithay::reexports::drm::control::{Device as ControlDevice, Mode, connector, crtc};

use self::heads::{HeadAction, Probed, replan};
use super::buffers::BufferPool;
use super::head::Head;
use super::{State, Tty, crtc_gamma_size, crtcs, gpu};
use crate::compositor::headless;
use crate::compositor::render::ScanoutHandoff;

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
    let changes = {
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
                Vec::new()
            }
            // Every other device's events, and `Added` for any device: a new
            // GPU appearing on the seat is not something a single-device
            // backend can use.
            _ => Vec::new(),
        }
    };
    apply(state, changes);
}

/// One thing a re-probe changed, as `State` has to hear about it once the
/// borrow of `state.tty` that produced it has ended.
pub(super) enum Change {
    /// Something happened to an existing head's output (see
    /// [`Reconfigured`]).
    Output(OutputId, Reconfigured),
    /// The head presenting this output was torn down -- its connector went
    /// away and it was not the last screen -- so the output goes too.
    Removed(OutputId),
    /// A newly connected connector has a head (already in `Tty::heads`, its
    /// surface built) that still needs its output: created here, then
    /// attached by connector.
    Added {
        connector: connector::Handle,
        name: String,
        width: i32,
        height: i32,
        scanout: ScanoutHandoff,
    },
}

/// Hands every [`Change`] of one re-probe to the rest of `State`, in the
/// order that keeps each step meaningful: existing outputs follow their new
/// modes first, removed outputs go next (so a CRTC or position they freed is
/// free for what follows), and new outputs are created last, side by side to
/// the right of what remains. Called from [`udev_event`] and from
/// `session_event`'s reactivation arm.
pub(super) fn apply(state: &mut State, changes: Vec<Change>) {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for change in changes {
        match change {
            Change::Output(id, outcome) => outcome.finish(state, id),
            Change::Removed(id) => removed.push(id),
            Change::Added {
                connector,
                name,
                width,
                height,
                scanout,
            } => added.push((connector, name, width, height, scanout)),
        }
    }
    for id in removed {
        tracing::info!(
            output = id.0,
            "drm: a display went away; removing its output"
        );
        if !state.remove_output(id) {
            // Unreachable: the head planner never removes the last screen,
            // and every head presents one output. error!: the head is already
            // gone, so this output would show nothing from here on.
            tracing::error!(
                output = id.0,
                "drm: could not remove the output of a display that went away"
            );
        }
    }
    for (connector, name, width, height, scanout) in added {
        match headless::add_output_with(state, &name, width, height, scanout) {
            Ok(id) => {
                tracing::info!(
                    connector = %name,
                    output = id.0,
                    width,
                    height,
                    "drm: a display was connected; added an output for it"
                );
                super::attach_where(state, |head| head.connector == connector, id);
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    connector = %name,
                    "could not create an output for a newly connected display; leaving it dark"
                );
            }
        }
    }
    // A head whose output could not be created is dropped here rather than
    // left driving a CRTC with nothing to show.
    super::retain_attached(state);
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
    /// Does whatever this outcome owes the rest of `State` for output `id`
    /// (the head it happened to). Called once, after the `&mut Tty` borrow
    /// that produced it has ended -- from [`udev_event`] and from
    /// `session_event`'s reactivation arm.
    pub(super) fn finish(self, state: &mut State, id: OutputId) {
        match self {
            Self::Nothing => {}
            Self::Render => state.request_render(),
            Self::Resized(width, height) => apply_resize(state, id, width, height),
            Self::SwitchedCrtc {
                width,
                height,
                size_changed,
                gamma_size,
            } => {
                state.gamma_control.crtc_changed(id, gamma_size);
                if size_changed {
                    apply_resize(state, id, width, height);
                } else {
                    state.request_render();
                }
            }
        }
    }
}

/// Moves output `id`'s render target and everything downstream of it onto a
/// new mode size: the shared body of `Reconfigured::{Resized,
/// SwitchedCrtc}`'s size-changing arms.
fn apply_resize(state: &mut State, id: OutputId, width: i32, height: i32) {
    if !state.resize_output_of(id, width, height) {
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
            output = id.0,
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
/// without a DRM device: `probed` is what a re-probe found for a head
/// (`None` when it is not `Connected` any more), `current` is the connector
/// and physical size that head is on right now. `heads::replan` asks it for
/// each head that is still connected.
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
fn plan<C: PartialEq>(current: (C, (i32, i32)), probed: Option<(C, (i32, i32))>) -> Plan {
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
    /// driven -- for every head at once, and for every connector no head
    /// drives yet (see [`heads::replan`] for the rules). Answers what the rest
    /// of `State` owes each change, in [`apply`]'s terms.
    ///
    /// Every connector is re-probed (forced -- see `gpu::Freshness`), not only
    /// the driven ones: a monitor plugged in is only visible to a probe of
    /// its own connector. That is one EDID read per connected connector per
    /// uevent, the same cost wlroots pays on the same path; a disconnected
    /// connector answers without reading anything.
    ///
    /// Called from [`udev_event`] on a `change` uevent for this device, and
    /// from `session_event`'s `ActivateSession` arm after a successful
    /// reactivation -- a monitor plugged in while the session was
    /// VT-switched away produces its uevent then, when this backend has no
    /// DRM master and cannot act on it, so the switch back has to ask again
    /// rather than assume nothing moved.
    pub(super) fn reconfigure(&mut self) -> Vec<Change> {
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
            return Vec::new();
        }
        // Read fresh, not cached from startup: the whole point is that the
        // set of connectors and their modes has changed underneath us.
        let resources = match self.drm.resource_handles() {
            Ok(resources) => resources,
            Err(error) => {
                tracing::warn!(%error, "drm: could not re-read the device's resources after a hotplug");
                return Vec::new();
            }
        };
        let requested = self.requested_mode;
        let driven: Vec<Option<gpu::Connected>> = self
            .heads
            .iter()
            .map(|head| {
                gpu::connector_mode(
                    &self.drm,
                    &resources,
                    head.connector,
                    requested,
                    gpu::Freshness::Reprobe,
                )
            })
            .collect();
        let undriven: Vec<gpu::Connected> = resources
            .connectors()
            .iter()
            .copied()
            .filter(|conn| !self.heads.iter().any(|head| head.connector == *conn))
            .filter_map(|conn| {
                gpu::connector_mode(
                    &self.drm,
                    &resources,
                    conn,
                    requested,
                    gpu::Freshness::Reprobe,
                )
            })
            .collect();
        let probed: Vec<Probed<connector::Handle>> = self
            .heads
            .iter()
            .zip(&driven)
            .map(|(head, now)| Probed {
                connector: head.connector,
                size: (head.width, head.height),
                now: now.as_ref().map(|found| mode_size(found.mode)),
            })
            .collect();
        let candidates: Vec<(connector::Handle, (i32, i32))> = undriven
            .iter()
            .map(|found| (found.connector, mode_size(found.mode)))
            .collect();
        let plan = replan(&probed, &candidates, self.nothing_connected);

        let mut changes = Vec::new();
        let crtcs: Vec<crtc::Handle> = self
            .heads
            .iter()
            .map(|head| head.presenter.crtc())
            .collect();
        let mut remove: Vec<usize> = Vec::new();
        for (index, action) in plan.heads.iter().enumerate() {
            let Tty {
                drm,
                heads,
                nothing_connected,
                ..
            } = self;
            let Some(head) = heads.get_mut(index) else {
                continue;
            };
            let Some(id) = head.output else {
                continue;
            };
            let own = head.presenter.crtc();
            let others: Vec<crtc::Handle> =
                crtcs.iter().copied().filter(|&crtc| crtc != own).collect();
            let outcome = match *action {
                HeadAction::Keep => {
                    tracing::debug!(
                        connector = %head.name,
                        "drm: hotplug changed nothing this head is driving"
                    );
                    Reconfigured::Nothing
                }
                HeadAction::NewMode(_) => match driven.get(index).and_then(Option::as_ref) {
                    Some(found) => {
                        head.retarget(drm, &others, found.connector, found.mode, &found.name)
                    }
                    None => Reconfigured::Nothing,
                },
                HeadAction::Reconnected => {
                    // The undock/redock case: the same monitor came back at
                    // the same mode, so nothing about the *choice* changed --
                    // but the CRTC was driving a connector that physically
                    // went away in between, and whether it lights back up
                    // without a modeset is the driver's business, not
                    // something to bet a black screen on. Force one.
                    tracing::info!(
                        connector = %head.name,
                        "drm: a display is connected again; forcing a modeset"
                    );
                    head.presenter.invalidate_scanout();
                    Reconfigured::Render
                }
                HeadAction::MoveTo(target, _) => {
                    match undriven.iter().find(|found| found.connector == target) {
                        Some(found) => {
                            head.retarget(drm, &others, found.connector, found.mode, &found.name)
                        }
                        None => Reconfigured::Nothing,
                    }
                }
                HeadAction::Hold => {
                    // Once per disconnection, not once per uevent: a display
                    // being unplugged usually produces several `change`
                    // events in a row, and this is a state the session can
                    // sit in for hours.
                    if !std::mem::replace(nothing_connected, true) {
                        // warn!, not error!: nothing is broken and nothing is
                        // lost -- there is simply no display to drive.
                        // Plugging one back in recovers on its own, which is
                        // exactly what the message has to say, because the old
                        // behaviour here (a black screen until restart) taught
                        // the opposite.
                        tracing::warn!(
                            "drm: nothing is connected to this device any more; \
                             holding the last frame. The session keeps running -- \
                             plug a display back in and scoot mode-sets onto it."
                        );
                    }
                    Reconfigured::Nothing
                }
                HeadAction::Remove => {
                    remove.push(index);
                    Reconfigured::Nothing
                }
            };
            // The primary's hold ends only once a move or modeset onto a
            // display actually *took*: clearing it before would throw away the
            // one thing that makes the next identical uevent retry rather than
            // plan `Keep` and do nothing (a refused buffer allocation, a
            // surface that would not take the new state).
            if index == 0
                && matches!(
                    action,
                    HeadAction::Reconnected | HeadAction::MoveTo(..) | HeadAction::NewMode(_)
                )
                && outcome != Reconfigured::Nothing
            {
                *nothing_connected = false;
            }
            if outcome != Reconfigured::Nothing {
                changes.push(Change::Output(id, outcome));
            }
        }
        // Highest index first, so every index still names its head. Dropping
        // a head drops its surface, whose `Drop` clears that CRTC -- its
        // connector is gone, and the CRTC is free for a head built below.
        for index in remove.into_iter().rev() {
            let head = self.heads.remove(index);
            if head.presenter.flip_in_flight() {
                self.stale_vblanks.push(head.presenter.crtc());
            }
            tracing::info!(connector = %head.name, "drm: this connector went away");
            if let Some(id) = head.output {
                changes.push(Change::Removed(id));
            }
        }
        for (target, _) in &plan.add {
            let Some(found) = undriven.iter().find(|found| found.connector == *target) else {
                continue;
            };
            if let Some(change) = self.add_head(found) {
                changes.push(change);
            }
        }
        changes
    }

    /// Builds a head for a connector that was just plugged in: a CRTC it can
    /// reach that no lit head holds (lit heads are never re-routed to make
    /// room), a surface on it, and a presenter on the session's own tier.
    /// `None` -- with a warning -- when it cannot be driven: already at
    /// `MAX_OUTPUTS`, no free CRTC, a surface or presenter that refuses. The
    /// screen stays dark and nothing else changes.
    fn add_head(&mut self, found: &gpu::Connected) -> Option<Change> {
        if self.heads.len() >= crate::cli::MAX_OUTPUTS as usize {
            tracing::warn!(
                connector = %found.name,
                max = crate::cli::MAX_OUTPUTS,
                "drm: already driving the most outputs scoot supports; leaving this connector dark"
            );
            return None;
        }
        let busy: Vec<crtc::Handle> = self
            .heads
            .iter()
            .map(|head| head.presenter.crtc())
            .collect();
        let reachable = if found.crtcs.is_empty() {
            self.drm.crtcs().to_vec()
        } else {
            found.crtcs.clone()
        };
        let order: Vec<crtc::Handle> = crtcs::assign(std::slice::from_ref(&reachable), &busy)
            .into_iter()
            .flatten()
            .chain(
                reachable
                    .iter()
                    .copied()
                    .filter(|crtc| !busy.contains(crtc)),
            )
            .collect();
        if order.is_empty() {
            tracing::warn!(
                connector = %found.name,
                "drm: no free crtc can drive this newly connected display; leaving it dark"
            );
            return None;
        }
        let Some(surface) = super::create_surface(
            &mut self.drm,
            order.into_iter(),
            found.connector,
            found.mode,
        ) else {
            tracing::warn!(
                connector = %found.name,
                "drm: no crtc would take a surface for this newly connected display; leaving it dark"
            );
            return None;
        };
        let fd = self.drm.device_fd().clone();
        let tier = self.renderer();
        match super::build_head(&mut self.drm, &fd, surface, found, Some(tier), tier) {
            Ok((head, scanout, _)) => {
                let change = Change::Added {
                    connector: head.connector,
                    name: head.name.clone(),
                    width: head.width,
                    height: head.height,
                    scanout,
                };
                tracing::info!(
                    connector = %head.name,
                    crtc = ?head.presenter.crtc(),
                    width = head.width,
                    height = head.height,
                    scanout = head.presenter.tier(),
                    "drm: driving a newly connected display"
                );
                self.heads.push(head);
                Some(change)
            }
            Err(reason) => {
                tracing::warn!(connector = %found.name, %reason, "drm: leaving this connector dark");
                None
            }
        }
    }
}

impl Head {
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
    /// Order is load-bearing. Whatever the new mode needs is allocated
    /// *first*, before anything on the DRM side is told to change: a failed
    /// allocation then leaves the display exactly as it was and working,
    /// whereas the other order would leave the CRTC on a mode whose frames no
    /// buffer is the right size to hold -- which `present`'s size guard drops
    /// silently, forever. See `Presenter::new_buffers` for what each tier
    /// allocates (the scanout tier: nothing, by design).
    fn retarget(
        &mut self,
        drm: &mut DrmDevice,
        others: &[crtc::Handle],
        connector: connector::Handle,
        mode: Mode,
        name: &str,
    ) -> Reconfigured {
        let (width, height) = mode_size(mode);
        let size_changed = (width, height) != (self.width, self.height);
        let Ok(buffers) = self.presenter.new_buffers(drm, size_changed, width, height) else {
            return Reconfigured::Nothing;
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
            return self.switch_crtc(drm, others, connector, mode, name, buffers, size_changed);
        }
        tracing::info!(
            connector = %name,
            width,
            height,
            "drm: display reconfigured; mode-setting onto it"
        );
        // Before `self.width`/`height` and before the `wl_output`'s own mode
        // moves (`Reconfigured::finish` -> `State::resize_output` ->
        // `set_mode`, all of which run after this function returns). On the
        // scanout tier that leaves a window in which the swapchain is the new
        // size while the compositor's `OutputModeSource::Auto(output)` still
        // reports the old one -- harmless only because nothing renders in
        // between: this runs inside the udev handler's borrow of `state.tty`,
        // and `finish` runs immediately after it, with no event-loop dispatch
        // (and so no frame) between the two.
        if !self.presenter.adopt_mode(mode) {
            // Scanout tier only, and all but unreachable: `set_pending` has
            // just put this exact mode on this exact surface, so the repeat
            // inside `DrmCompositor::use_mode` is asking a question that was
            // answered `Ok` a line ago. error!, not warn!, because if it ever
            // does happen the surface is on the new mode while the swapchain
            // is still sized for the old one, and every frame from then on
            // would be the wrong size. Returning `Nothing` leaves
            // `self.width`/`height` untouched, so the next uevent plans
            // `NewMode` again and retries rather than believing the move
            // landed.
            tracing::error!(
                connector = %name, width, height,
                "drm: the surface took the new mode but the scanout compositor \
                 would not; staying on the current one"
            );
            return Reconfigured::Nothing;
        }
        self.connector = connector;
        // The old pool -- and the framebuffers in it, one of which the CRTC
        // may still be scanning out -- is dropped inside `adopt_buffers`; see
        // its doc for what the kernel does with a framebuffer removed while
        // active. `None` (the size did not change, or the scanout tier, which
        // has no pool) keeps whatever is there.
        self.presenter.adopt_buffers(buffers);
        // Unconditionally, not inside `adopt_buffers`: these two are
        // `present`'s size guard and must equal the size the buffers were
        // built at, whichever branch got here. Writing them only where a new
        // pool was built would make that a fact about that `if`, which is
        // exactly the kind of coupling that survives one refactor and not
        // two. When the size did not change they are already these values.
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
    #[allow(clippy::too_many_arguments)]
    fn switch_crtc(
        &mut self,
        drm: &mut DrmDevice,
        others: &[crtc::Handle],
        connector: connector::Handle,
        mode: Mode,
        name: &str,
        buffers: Option<BufferPool>,
        size_changed: bool,
    ) -> Reconfigured {
        let (width, height) = mode_size(mode);
        let current = self.presenter.crtc();
        // Copied, not borrowed: `create_surface` below needs `&mut drm`.
        // One small `Vec` on a path that runs when a cable moves, matching
        // `tty::create_surface`'s own shape. Never another head's CRTC
        // (`others`): Smithay would refuse it anyway (its primary plane is
        // claimed), but a lit screen is not a candidate to probe.
        let crtcs: Vec<crtc::Handle> = drm
            .crtcs()
            .iter()
            .copied()
            .filter(|&crtc| crtc != current && !others.contains(&crtc))
            .collect();
        for crtc in crtcs {
            let candidate = match drm.create_surface(crtc, mode, &[connector]) {
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
            //
            // On the scanout tier this rebuilds the whole `DrmCompositor`
            // (it owns its surface by value) and can refuse. It builds the
            // replacement before dropping the live one, so a refusal leaves
            // this CRTC, this connector and what is on screen exactly as they
            // were -- the same property the loop relies on for a candidate
            // that fails earlier -- and the next candidate is tried.
            if !self.presenter.adopt_surface(candidate) {
                continue;
            }
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
                gamma_size: crtc_gamma_size(drm, self.presenter.crtc()),
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
