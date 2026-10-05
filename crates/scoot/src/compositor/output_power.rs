//! `zwlr_output_power_manager_v1`: turning screens off when idle.
//!
//! The other half of the idle story `idle.rs` starts: a `swayidle`-style
//! daemon learns the seat is idle through `ext-idle-notify-v1`, locks the
//! session through `ext-session-lock-v1`, and powers the panels down through
//! this protocol -- the same `swayidle timeout 600 'wlopm --off *' resume
//! 'wlopm --on *'` setup every other compositor runs. scoot implements the
//! standard protocol rather than a bespoke one (see CLAUDE.md), plus a
//! session-level IPC request (`output-power`) for agents.
//!
//! Smithay carries no helper for this protocol at the pinned rev, so like
//! `wlr-gamma-control-v1` (`gamma_control.rs`) this is hand-implemented
//! against the `output_power_management` server bindings from
//! `wayland-protocols-wlr` (re-exported through Smithay, so the types are
//! the same ones the rest of the compositor dispatches). The shape follows
//! the same model: a `GlobalData` created once in [`OutputPower::new`], a
//! per-bound-manager `UserData`, and `Dispatch2`/`GlobalDispatch2` impls
//! that scoot's blanket `Dispatch` (see `dispatch.rs`) forwards to with no
//! per-interface code of its own.
//!
//! ## Trust model
//!
//! The global is visible to every client (`can_view` is unconditional).
//! scoot has no security-context support to distinguish a privileged idle
//! daemon from any other client, so an allow-list would be theatre -- the
//! same rationale as the session-lock and gamma globals, documented in
//! `site/src/content/docs/scoot/protocols.md`'s trust note.
//!
//! ## What "off" means
//!
//! The compositor-side state ([`OutputPower::off`], keyed by the core id, so
//! a replugged monitor -- which always comes back under a fresh id -- comes
//! back on) is authoritative, and every backend honours it the same way:
//! the render loop (`headless.rs`) skips a powered-off output wholesale --
//! no draw, no present, no frame callbacks, no presentation feedback -- so an
//! idle session with dark screens does ~zero render work. What the client
//! committed meanwhile is not lost: Smithay's damage history only advances
//! on a drawn frame, so the first frame after power-on repaints whatever
//! accumulated, and a client starved of callbacks simply paints late. The
//! pointer and keyboard keep working while off, and input by itself never
//! turns a screen back on -- that is the idle daemon's `resume` job, over
//! this protocol or over IPC.
//!
//! The hardware half runs only under `--tty`: the DPMS connector property
//! (`Tty::set_power`), `On`/`Off` only. Under `--headless`/`--nested` there
//! is no panel to power down, so the request succeeds, the mode is tracked
//! and reported honestly, and the render work is skipped -- and an IPC
//! screenshot of a powered-off output is refused rather than answered from
//! its stale framebuffer, while a parked `ext-image-copy-capture-v1` frame
//! due on one fails with `unknown` (see `screencopy.rs`).
//!
//! ## Lock, hotplug and VT switching
//!
//! - **Session lock.** Power state is orthogonal to it: locking blanks the
//!   screens that are on and leaves the dark ones dark. A powered-off output
//!   counts as blanked for lock confirmation (see
//!   [`SessionLock::note_powered_off`](super::session_lock::SessionLock::note_powered_off)):
//!   a dark screen *is* blank, and waiting for a frame that will never be
//!   drawn would hang the locker on the fallback timeout. Powering back on
//!   while locked resumes drawing locked frames, which confirm the usual way.
//! - **Hotplug.** Removing an output fails every power object for it (the
//!   protocol's answer for an output that went away) and forgets its
//!   power state; the replugged monitor starts on. See
//!   [`OutputPower::forget_output`], called from `remove_output` beside
//!   gamma's.
//! - **VT switch.** Switching away releases DRM master, so the DPMS state
//!   cannot be vouched for on the way back; reactivation re-applies the
//!   hardware half for every output still off (see
//!   [`State::reapply_output_power`]).
//!
//! ## Protocol notes
//!
//! Unlike gamma there is no exclusivity transfer: the protocol defines none,
//! so any number of clients may hold a control for one output and the last
//! `set_mode` wins. Every `mode` event goes to every object for that output,
//! including the ones other clients hold -- and including when scoot itself
//! changed the mode (the IPC request broadcasts the same way).
//!
//! A `failed` event means the object is dead: the output went away, or the
//! hardware refused the change (no DRM master after a VT switch, a driver
//! without a DPMS property). The compositor-side state still applies --
//! render work stays skipped -- and a client that re-reads the mode after
//! re-creating its object sees the current one.

use scoot_core::OutputId;

use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1,
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1,
    zwlr_output_power_v1::{Mode as PowerMode, ZwlrOutputPowerV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

/// Holds the `zwlr_output_power_manager_v1` global alive, tracks every live
/// power object by the output it controls, and holds the compositor-side
/// power state itself.
///
/// Two collections because they answer different questions: `controls` is
/// who to tell (every object for an output, for `mode` broadcasts and
/// `failed`), `off` is what is true (the render loop's skip, the IPC
/// `outputs` field, the hardware re-apply). Both are `Vec`s, not maps: at
/// most eight outputs, read per request or per frame, never per pixel.
pub struct OutputPower {
    /// Held only to keep the manager global alive -- like
    /// `output_manager_state`, nothing reads this field again after `new`.
    #[allow(dead_code)]
    manager_global: GlobalId,
    /// Every live power object with the output it controls. Several clients
    /// may hold one for the same output (no exclusivity); a destroyed object
    /// is forgotten in `destroyed` below, a failed one at the `failed` site,
    /// so this never outlives the object.
    controls: Vec<(OutputId, ZwlrOutputPowerV1)>,
    /// The outputs currently off, by core id. Absent is on. Written only by
    /// [`State::set_output_powered`], cleared for an output by
    /// [`OutputPower::forget_output`], so a replugged monitor (fresh id,
    /// never reused) always starts on.
    off: Vec<OutputId>,
}

impl OutputPower {
    pub(super) fn new(display: &DisplayHandle) -> Self {
        let manager_global = display
            .create_global::<State, ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData>(
                1,
                OutputPowerManagerGlobalData,
            );
        Self {
            manager_global,
            controls: Vec::new(),
            off: Vec::new(),
        }
    }

    /// Whether output `id` is currently off. Absent from `off` is on --
    /// including an id this compositor never had, which reads as on rather
    /// than erroring: the render loop asks per output it holds, and IPC
    /// validates ids before it ever gets here.
    pub(super) fn is_off(&self, id: OutputId) -> bool {
        self.off.contains(&id)
    }

    /// Records `id`'s new state, reporting whether it changed. No events,
    /// no render requests here: the caller ([`State::set_output_powered`])
    /// owns those, so the protocol and IPC paths share them.
    fn set_off(&mut self, id: OutputId, off: bool) -> bool {
        if off == self.is_off(id) {
            return false;
        }
        if off {
            self.off.push(id);
        } else {
            self.off.retain(|known| *known != id);
        }
        true
    }

    /// The ids currently off, for the hardware re-apply after a VT switch
    /// back or a hotplug reconfigure. A slice, not a copy: at most eight
    /// entries, read on a cold path.
    pub(super) fn off_ids(&self) -> &[OutputId] {
        &self.off
    }

    /// Sends the current mode to every live object for output `id`. Called
    /// after every change however it arrived -- a client's `set_mode` or
    /// scoot itself (the IPC request) -- so every holder learns the mode,
    /// not just the one that asked.
    fn broadcast_mode(&self, id: OutputId) {
        let mode = if self.is_off(id) {
            PowerMode::Off
        } else {
            PowerMode::On
        };
        for (known, control) in &self.controls {
            if *known == id {
                control.mode(mode);
            }
        }
    }

    /// Files `control` as a live object for output `id`. No exclusivity:
    /// every client that asks holds one, and every one hears every change.
    fn track(&mut self, id: OutputId, control: ZwlrOutputPowerV1) {
        self.controls.push((id, control));
    }

    /// Fails every live object for output `id` and forgets them: the
    /// `failed` half of a refused hardware change (see
    /// [`State::set_output_powered`]) and of an output going away (see
    /// [`OutputPower::forget_output`).
    fn fail_output(&mut self, id: OutputId) {
        self.controls.retain(|(known, control)| {
            if *known == id {
                control.failed();
                false
            } else {
                true
            }
        });
    }

    /// Fails every live object for output `id` and forgets the output
    /// entirely: its controls and its power state. Called when the output
    /// goes away (`remove_output`), so a replugged monitor starts on under
    /// its fresh id.
    pub(super) fn forget_output(&mut self, id: OutputId) {
        self.fail_output(id);
        self.off.retain(|known| *known != id);
    }

    /// Which output's control `control` is, if it is still a live one. A
    /// failed or destroyed object is on no output: requests on it are a
    /// client bug, not state to apply.
    fn output_of(&self, control: &ZwlrOutputPowerV1) -> Option<OutputId> {
        self.controls
            .iter()
            .find(|(_, current)| *current == *control)
            .map(|(id, _)| *id)
    }

    /// How many live power objects this compositor holds. Test-only: the
    /// multi-client broadcast suites assert coexistence here rather than
    /// through a second round trip.
    #[cfg(test)]
    pub(super) fn live_control_count(&self) -> usize {
        self.controls.len()
    }
}

/// Global data for the manager. Empty: every client may bind (see the module
/// doc's trust model), so there is no filter to carry.
pub(super) struct OutputPowerManagerGlobalData;

impl GlobalDispatch2<ZwlrOutputPowerManagerV1, State> for OutputPowerManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, OutputPowerManagerUserData);
    }
}

/// Per-bound-manager user data. Empty: all state lives in [`OutputPower`],
/// reached through `&mut State`.
pub(super) struct OutputPowerManagerUserData;

impl Dispatch2<ZwlrOutputPowerManagerV1, State> for OutputPowerManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputPowerManagerV1,
        request: <ZwlrOutputPowerManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                get_output_power(state, data_init, id, &output);
            }
            zwlr_output_power_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}

/// Per-control user data. Empty, for the same reason as the manager's: the
/// live controls are tracked in [`OutputPower::controls`].
pub(super) struct OutputPowerUserData;

impl Dispatch2<ZwlrOutputPowerV1, State> for OutputPowerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrOutputPowerV1,
        request: <ZwlrOutputPowerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => {
                // Stale controls -- failed when their output went away, or
                // already failed on a refused change -- stop affecting
                // anything. (A client that destroys and re-creates gets a
                // fresh object, which *is* current, so this only drops
                // requests on dead objects.)
                if state.output_power.output_of(resource).is_none() {
                    return;
                }
                set_mode(state, resource, mode);
            }
            zwlr_output_power_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &ZwlrOutputPowerV1) {
        // A disconnect destroys the object without the request above ever
        // running. Power state persists -- destroying the last control for
        // an output changes nothing about the output -- so unlike gamma's
        // restore, this only forgets the object.
        state
            .output_power
            .controls
            .retain(|(_, current)| *current != *resource);
    }
}

/// Creates the per-output power object for `get_output_power`.
///
/// Always initializes `id` and always answers on the new object: the
/// current `mode` for an output this compositor has, `failed` for anything
/// else (an output that went away, or a request that arrived before the
/// first output exists). Initializing-then-failing, rather than posting a
/// protocol error without initializing, keeps this on the protocol's own
/// rails -- an output that cannot be controlled is exactly what `failed`
/// is for -- and avoids the never-initialized-object shape `dispatch.rs`
/// documents for the `wl_shm` guards.
fn get_output_power(
    state: &mut State,
    data_init: &mut DataInit<'_, State>,
    id: New<ZwlrOutputPowerV1>,
    output: &smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
) {
    let known = state
        .outputs
        .iter()
        .find(|known| known.owns(output))
        .and_then(|known| state.outputs.id_of(known));
    let control: ZwlrOutputPowerV1 = data_init.init(id, OutputPowerUserData);
    let Some(output_id) = known else {
        control.failed();
        return;
    };
    control.mode(if state.output_power.is_off(output_id) {
        PowerMode::Off
    } else {
        PowerMode::On
    });
    // Tracked after the initial event, moving rather than cloning: nothing
    // reads the list between the two.
    state.output_power.track(output_id, control);
}

/// Applies one `set_mode`.
///
/// Anything but `Off` or `On` is `invalid_mode` -- the protocol's own
/// error, which disconnects the client. That includes future modes a newer
/// protocol might define (which arrive as `Unknown`): this version knows two.
fn set_mode(state: &mut State, resource: &ZwlrOutputPowerV1, mode: WEnum<PowerMode>) {
    // The caller has already checked this control is live, so it names an
    // output; the `let else` is the proof, not a new refusal path.
    let Some(id) = state.output_power.output_of(resource) else {
        return;
    };
    let Some(on) = mode_to_on(mode) else {
        resource.post_error(
            zwlr_output_power_v1::Error::InvalidMode,
            format!("power mode must be off (0) or on (1), not {mode:?}"),
        );
        return;
    };
    state.set_output_powered(id, on);
}

/// Maps a `set_mode` argument onto the state it asks for: `None` is the
/// `invalid_mode` refusal. Pure so the refusal mapping is unit-pinned
/// below: no real client can send an invalid mode through the generated
/// bindings (they take the typed enum, and an out-of-range value panics
/// client-side before it reaches the wire), while a raw client (`wlopm`
/// with a bad argument) arrives here as `Unknown`.
fn mode_to_on(mode: WEnum<PowerMode>) -> Option<bool> {
    match mode {
        WEnum::Value(PowerMode::Off) => Some(false),
        WEnum::Value(PowerMode::On) => Some(true),
        _ => None,
    }
}

impl State {
    /// Sets output `id`'s power state -- the one path both the protocol
    /// (`set_mode`) and IPC (`output-power`) go through, so the two agree
    /// about events, hardware and rendering.
    ///
    /// A repeated set (off when already off) is a silent no-op: no events,
    /// no hardware poke, no render request -- the idle daemon re-asserting
    /// the state it wants must not churn. Otherwise, in order: record the
    /// state, attempt the `--tty` hardware half (a refusal fails this
    /// output's protocol objects but keeps the compositor-side state --
    /// render work stays skipped -- see the module doc), broadcast the mode
    /// to every holder, count the output as blanked if a lock is waiting
    /// (dark is blank), and ask for a render (the power-on repaint, or the
    /// no-op tick that clears the flag when every screen is dark).
    pub(crate) fn set_output_powered(&mut self, id: OutputId, on: bool) {
        // Both wire paths validate first -- the protocol resolves through
        // live controls only (`output_of`), IPC against `self.outputs` --
        // so an unknown id here is a caller bug, not a client spelling.
        // Loud in debug, a no-op in release, and never a phantom in `off`
        // that every later re-apply would warn about.
        if self.outputs.get(id).is_none() {
            debug_assert!(false, "set_output_powered for an unknown output");
            tracing::warn!(
                output = id.0,
                "ignoring a power change for an unknown output"
            );
            return;
        }
        if !self.output_power.set_off(id, !on) {
            return;
        }
        if let Some(tty) = self.tty.as_mut()
            && let Err(error) = tty.set_power(id, on)
        {
            tracing::warn!(output = id.0, %error, "could not set output power; failing its controls");
            self.output_power.fail_output(id);
        }
        self.output_power.broadcast_mode(id);
        if !on {
            self.count_powered_off_for_lock(id);
        }
        self.request_render();
    }

    /// Counts a powered-off output toward a pending lock confirmation (see
    /// [`SessionLock::note_powered_off`](super::session_lock::SessionLock::note_powered_off)):
    /// dark is blank. A no-op unless a lock is waiting.
    fn count_powered_off_for_lock(&mut self, id: OutputId) {
        if !self.session_lock.awaiting_blank() {
            return;
        }
        let expected = self.outputs.len();
        if self.session_lock.note_powered_off(id, expected) {
            self.confirm_lock();
        }
    }

    /// Re-applies the hardware half of the power state: every output still
    /// off is switched off again. Called after anything that re-modesets
    /// behind the state -- the VT-switch reactivation and the hotplug
    /// reconfigure, both through `hotplug::apply` -- which would otherwise
    /// leave a logically-off screen lit. Skipped without DRM master (a
    /// `set_property` there can only fail) and without a backend; hardware
    /// refusals are logged, never fatal.
    pub fn reapply_output_power(&mut self) {
        let held = self.tty.as_ref().is_some_and(|tty| tty.is_active());
        if !held {
            return;
        }
        let off: Vec<OutputId> = self.output_power.off_ids().to_vec();
        for id in off {
            if let Some(tty) = self.tty.as_mut()
                && let Err(error) = tty.set_power(id, false)
            {
                tracing::warn!(output = id.0, %error, "could not re-apply output power after reconfigure");
            }
        }
    }
}
