//! Virtual pointer and keyboard: remote control through standard protocols.
//!
//! A VNC server like wayvnc is a Wayland client like any other: it captures
//! the screen through [`ext-image-copy-capture-v1`](super::screencopy) and
//! drives the pointer and keyboard through `zwlr_virtual_pointer_manager_v1`
//! (version 2) and `zwp_virtual_keyboard_manager_v1` (version 1), which this
//! module implements by hand. Smithay has no virtual-pointer module at the
//! pinned rev, and its virtual-keyboard manager cannot carry scoot's policy
//! (its `Dispatch2` delivers straight to seat focus with no lock gate, no
//! per-source teardown and no activity announcement), so both live here
//! against the server bindings Smithay re-exports -- no new dependency, no
//! fork change (see `docs/forks.md` for what would justify one).
//!
//! ## Gating
//!
//! The globals exist only when `[virtual_input] enabled` is set (see
//! [`super::config`]): off means not advertised at all, so a client takes
//! its fallback path (wayvnc logs which protocol is missing) instead of
//! discovering a refusal at bind time. An allow-list would be theatre --
//! scoot has no security-context support, so any same-uid client that can
//! bind may drive (the trust note in `site/src/content/docs/scoot/protocols.md`) -- and a flag can
//! only say yes, so this takes effect on restart like `[xwayland] enabled`.
//!
//! ## Binds (opt-in, default off)
//!
//! Virtual keys never run keybindings unless `[virtual_input] binds` is set
//! alongside `enabled` (see [`super::config`]): off means forward-only,
//! like Smithay's own manager -- window management stays local. On, a
//! virtual key that matches the session's bind table by its *translated
//! seat keysym* runs the bind through the same `act_bind` path a physical
//! key does, and the press (and its matching release) is intercepted rather
//! than forwarded.
//!
//! Why the seat keysym, not the remote position: the remote layout and the
//! seat layout binds are configured against can disagree about what a key
//! *is* (wayvnc's `-k de` against a US seat puts `z` where the seat has
//! `y`). Matching by position would make remote keys behave like neither;
//! matching by the translated keysym -- the same keysym the forwarded
//! keystroke would carry -- makes a remote `Super+z` mean the seat's `z`
//! bind wherever `z` lives on either layout. A remote `Alt+Return` still
//! fires with a mismatched remote keymap, because `Return` is the same
//! keysym at every position that carries it.
//!
//! The rest of the contract, each pinned by test:
//!
//! - **Locked means neither.** Every mutating request is dropped while the
//!   session is locked (see below), so a virtual key never unlocks and
//!   never runs a bind while locked -- not even an `allow_when_locked`
//!   spawn. The press never reaches the filter; there is nothing to
//!   intercept and nothing forwarded.
//! - **No repeat.** A flagged bind re-fires while a physical key is held
//!   (see `bind_repeat.rs`); a virtual one fires exactly once. A virtual
//!   hold has no repeat lifecycle the compositor owns -- the client may
//!   disconnect mid-hold, and the destroy/lock/VT sweep must still release
//!   it cleanly -- so arming the timer would risk re-firing after the key
//!   is gone. Fire-once is the safe subset.
//! - **One fire across sources.** Held state is tracked per source inside
//!   Smithay: a second source pressing a seat keycode another already
//!   holds is absorbed before the filter (no re-fire, no duplicate
//!   forward). A virtual and a physical press of the same key therefore
//!   fire at most once, whichever transitioned first.
//! - **No cross-talk with physical suppression.** Intercepted virtual
//!   presses are recorded in `State::virtual_suppressed` -- never in
//!   `suppressed_keys`, which stays the physical path's alone -- so neither
//!   side can swallow the other's release. Two virtual keyboards holding
//!   one key share the one entry the same way Smithay shares the hold
//!   (the filter runs only on the first press and the last release), so
//!   the bind still fires exactly once.
//! - **Focus follows the seat.** Delivery (and interception) is through the
//!   seat to whoever has focus, exactly like a physical key: a bind that
//!   closes the focused window closes the focused window, and a forwarded
//!   key types into it.
//! - **Off is inert.** The filter's first check is the one bool
//!   (`State::virtual_input_binds`); off means `Forward` with no table
//!   lookup, no allocation, no state touched -- the physical per-keypress
//!   path never reads it at all.
//!
//! The trust note: any same-uid client that can bind the virtual keyboard
//! can already type into any window; with `binds` on it can also spawn
//! programs through binds (e.g. a terminal). That is the same trust
//! boundary `protocols.md` already states (no security-context support);
//! keep the flag off except for a webtop/VNC session the remote user owns.
//!
//! ## While locked
//!
//! Every mutating request (motion, buttons, scroll, keys, modifiers) is
//! dropped while the session is locked: a remote client must never unlock,
//! not even by typing the password blind into the lock screen. Keymap
//! uploads are harmless state and still accepted. Keys and buttons a device
//! holds when the lock engages are released first (see
//! [`State::release_virtual_input`], called from `lock_transition`), so
//! unlocking never finds a stuck modifier.
//!
//! ## Keyboards bring their own keymap
//!
//! A virtual keyboard's keycodes are positions in *its* keymap, which may
//! differ from the seat's (wayvnc's `-k de` against a US seat). Clients must
//! decode what the seat layout says -- the seat keymap is what every
//! `wl_keyboard` was sent -- so each key is translated by keysym: the keysym
//! the virtual keymap gives the pressed position becomes the seat keycode
//! carrying that keysym ([`KeyboardHandle::keycode_for_keysym`]). A keysym
//! the seat layout has no key for (a German `ß` on a US seat, anything past
//! the AltGr level the seat lacks) is dropped with a debug log rather than
//! mistyped: the honest subset of "type what the remote layout says".
//!
//! Delivery then goes through
//! [`KeyboardHandle::input_from_source`] with a per-device
//! [`KeyboardSource::Auxiliary`] id and -- unless `[virtual_input] binds`
//! is on -- a forward-only filter. That keeps
//! one seat-xkb invariant whole: held state is tracked per source inside
//! Smithay, and teardown -- run when a device is
//! destroyed, when the session locks and on a VT switch-away -- releases
//! the outstanding presses to whoever has focus (synthesized through the
//! same bind-aware filter when `binds` is on, so an intercepted press's
//! release is swallowed rather than forwarded as a lone release the client
//! never pressed; `release_source` when off, where nothing was ever
//! intercepted). With `binds` off, virtual keys never run keybindings
//! (forward-only, like Smithay's own manager) and nothing virtual ever
//! triggers a compositor action -- window management stays local. The seat keymap itself is never touched, so the
//! physical keyboard's held keys survive remote typing untouched -- but a
//! held virtual modifier does change the seat's shared modifier state until
//! it is released (a translated modifier press goes through
//! [`KeyboardHandle::input_from_source`], whose key processing updates the
//! shared xkb modifiers; Smithay's own keyboard manager behaves the same).
//!
//! ## Pointers
//!
//! Relative motion and absolute motion both end in the same absolute core
//! the other sources use ([`State::pointer_move`]: activity, focus, motion,
//! clamping included). Absolute coordinates map onto the device's bound
//! output when `create_virtual_pointer_with_output` named one the compositor
//! knows, and onto the output union otherwise (an unknown output falls back
//! there rather than killing the client: outputs come and go, and the
//! fallback is always mappable). A zero extent maps to the target's origin
//! rather than dividing by zero.
//!
//! Scroll arrives as several events closed by `frame` (source, continuous
//! and discrete values, stops), buffered in the shared [`PendingAxis`]
//! accumulator and emitted as one [`AxisFrame`](smithay::input::pointer::AxisFrame)
//! per frame -- the same shape the `--nested` host path uses, so a wheel
//! click means the same detents from either. Buttons go through
//! [`State::pointer_button`], so focus, serials, popup dismissal and
//! floating-drag settling behave exactly like a physical click; buttons the
//! compositor has no name for are dropped with a debug log, and
//! disconnecting with a button held synthesizes its release.
//!
//! A destroyed device releases what it held; a client that disappears takes
//! all its devices with it through the same path (destroyed fires per
//! object). No heap allocation happens per event: devices and keymaps live
//! per object, scroll accumulates in a stack struct, and the hot paths
//! (`motion`, `frame`, `key`) only look up and translate.
//!
//! [`KeyboardHandle::input_from_source`]: smithay::input::keyboard::KeyboardHandle::input_from_source
//! [`KeyboardHandle::release_source`]: smithay::input::keyboard::KeyboardHandle::release_source
//! [`KeyboardHandle::keycode_for_keysym`]: smithay::input::keyboard::KeyboardHandle::keycode_for_keysym
//! [`PendingAxis`]: super::nested::PendingAxis

use std::collections::HashMap;
use std::os::unix::io::OwnedFd;

use scoot_ipc::PointerButton;

use smithay::backend::input::{AxisSource, InputTime, KeyState};
use smithay::input::keyboard::{
    FilterResult, KeyboardSource, Keycode, KeysymHandle, ModifiersState, xkb,
};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_pointer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::reexports::wayland_server::{backend::GlobalId, protocol::wl_keyboard::KeymapFormat};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;
use super::keybindings::Bound;
use super::nested::PendingAxis;

#[cfg(test)]
mod tests;

/// Holds the two manager globals alive and tracks the live devices, one
/// entry per object. Created empty in [`State::new`]; [`init`] advertises
/// the globals when `[virtual_input] enabled` is set. Nothing here is read
/// on a frame or per physical event -- only virtual requests touch it.
pub struct VirtualInputState {
    /// Held only to keep the manager globals alive. `None` when the session
    /// runs with `[virtual_input]` off: no globals, no binds, no devices.
    #[allow(dead_code)]
    pointer_manager: Option<GlobalId>,
    /// Same, for the keyboard manager.
    #[allow(dead_code)]
    keyboard_manager: Option<GlobalId>,
    /// One live virtual pointer per object. Compared by object identity on
    /// destroy, so tearing down one device can never drop another's pending
    /// scroll or held buttons.
    pointers: HashMap<ZwlrVirtualPointerV1, VirtualPointer>,
    /// One live virtual keyboard per object, each with its own keymap and
    /// its own [`KeyboardSource::Auxiliary`] id for the seat's per-source
    /// held tracking.
    keyboards: HashMap<ZwpVirtualKeyboardV1, VirtualKeyboard>,
}

impl VirtualInputState {
    pub(super) fn new() -> Self {
        Self {
            pointer_manager: None,
            keyboard_manager: None,
            pointers: HashMap::new(),
            keyboards: HashMap::new(),
        }
    }
}

/// Advertise the two managers when `enabled` (`[virtual_input] enabled`).
/// Called once from `compositor::run`, after the outputs exist (absolute
/// motion maps onto them) and before `WAYLAND_DISPLAY` is exported, so no
/// client can bind before the globals it should see exist. Tests call this
/// directly on their harness state for the same reason.
pub(super) fn init(state: &mut State, enabled: bool) {
    if !enabled {
        return;
    }
    let dh = state.display_handle.clone();
    state.virtual_input.pointer_manager = Some(
        dh.create_global::<State, ZwlrVirtualPointerManagerV1, VirtualPointerManagerGlobalData>(
            2,
            VirtualPointerManagerGlobalData,
        ),
    );
    state.virtual_input.keyboard_manager = Some(
        dh.create_global::<State, ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerGlobalData>(
            1,
            VirtualKeyboardManagerGlobalData,
        ),
    );
}

/// One virtual pointer: where its absolute motion maps, what its scroll
/// sequence has buffered, and which buttons it currently holds.
struct VirtualPointer {
    /// The output `create_virtual_pointer_with_output` named, if any. Held
    /// as the client's own object; resolved to a rectangle per motion, so an
    /// output that went away (or one this compositor never had) simply stops
    /// resolving and motion falls back to the union.
    output: Option<WlOutput>,
    /// The scroll sequence in progress, flushed by `frame`. Shared with the
    /// `--nested` host path, so detents mean the same from either.
    pending: PendingAxis,
    /// Buttons this device holds, as a bit per [`PointerButton`]
    /// discriminant. At most the five the compositor names can be set (any
    /// other code is dropped at press time), so a fixed mask -- never an
    /// allocation -- covers every release and the disconnect/lock/VT sweep.
    held: u32,
}

/// One virtual keyboard: its own keymap and xkb state for keysym
/// translation, and the seat source its keys are attributed to.
struct VirtualKeyboard {
    /// This device's keymap and translation state. `None` until the client
    /// uploads one; keys and modifiers before that are the protocol's
    /// `no_keymap` error, not silent drops.
    keymap: Option<VirtualKeymap>,
    /// The seat source this device's keys are attributed to
    /// (`input_from_source`, and `release_source` on the binds-off path),
    /// minted per object.
    source: KeyboardSource,
    /// The seat keycodes this device currently holds, as the virtual evdev
    /// positions that produced them: press-time translation, not a
    /// re-translation on release (the device mask or group may have moved
    /// in between, and Smithay holds what the press sent). Only tracked so
    /// the binds-on teardown can synthesize releases through the bind-aware
    /// filter -- which swallows an intercepted press's release instead of
    /// forwarding a lone release the client never pressed. Read only on the
    /// virtual path and its teardown; the physical hot path never touches
    /// it.
    held: Vec<(u32, Keycode)>,
}

/// A virtual keyboard's own keymap: what its keycodes mean, independent of
/// the seat's. Never sent to clients and never installed on the seat -- it
/// exists only to translate positions to keysyms (see the module doc).
/// Only the `State` is kept: `xkb_state_new` holds its own reference on the
/// keymap, so the compiled map lives exactly as long as the translation
/// state derived from it, with no second allocation per device.
struct VirtualKeymap {
    state: xkb::State,
}

/// Global data for the pointer manager. Empty: the globals exist only when
/// `[virtual_input]` is on, so every client may bind (see the module doc's
/// gating).
struct VirtualPointerManagerGlobalData;

impl GlobalDispatch2<ZwlrVirtualPointerManagerV1, State> for VirtualPointerManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, VirtualPointerManagerUserData);
    }

    fn can_view(&self, _client: &Client) -> bool {
        true
    }
}

/// Per-bound-manager user data. Empty: devices live in
/// [`VirtualInputState`], reached through `&mut State`.
struct VirtualPointerManagerUserData;

impl Dispatch2<ZwlrVirtualPointerManagerV1, State> for VirtualPointerManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrVirtualPointerManagerV1,
        request: <ZwlrVirtualPointerManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { seat, id } => {
                create_pointer(state, data_init, seat, None, id);
            }
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                seat,
                output,
                id,
            } => {
                create_pointer(state, data_init, seat, output, id);
            }
            zwlr_virtual_pointer_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}

/// Global data for the keyboard manager. Empty, for the same reason as the
/// pointer manager's.
struct VirtualKeyboardManagerGlobalData;

impl GlobalDispatch2<ZwpVirtualKeyboardManagerV1, State> for VirtualKeyboardManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, VirtualKeyboardManagerUserData);
    }

    fn can_view(&self, _client: &Client) -> bool {
        true
    }
}

/// Per-bound-manager user data. Empty, for the same reason.
struct VirtualKeyboardManagerUserData;

impl Dispatch2<ZwpVirtualKeyboardManagerV1, State> for VirtualKeyboardManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardManagerV1,
        request: <ZwpVirtualKeyboardManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { id, .. } => {
                let keyboard: ZwpVirtualKeyboardV1 = data_init.init(id, VirtualKeyboardUserData);
                state.virtual_input.keyboards.insert(
                    keyboard,
                    VirtualKeyboard {
                        keymap: None,
                        source: KeyboardSource::new_auxiliary(),
                        held: Vec::new(),
                    },
                );
            }
            _ => unreachable!(),
        }
    }
}

/// Per-pointer user data. Empty: the device lives in
/// [`VirtualInputState::pointers`], reached through `&mut State`.
struct VirtualPointerUserData;

impl Dispatch2<ZwlrVirtualPointerV1, State> for VirtualPointerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrVirtualPointerV1,
        request: <ZwlrVirtualPointerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_virtual_pointer_v1::Request::Motion { dx, dy, .. } => {
                pointer_relative(state, resource, dx, dy);
            }
            zwlr_virtual_pointer_v1::Request::MotionAbsolute {
                x,
                y,
                x_extent,
                y_extent,
                ..
            } => {
                pointer_absolute(state, resource, x, y, x_extent, y_extent);
            }
            zwlr_virtual_pointer_v1::Request::Button {
                button,
                state: button_state,
                ..
            } => {
                pointer_button(state, resource, button, button_state);
            }
            zwlr_virtual_pointer_v1::Request::Axis { axis, value, .. } => {
                pointer_axis(state, resource, axis, value);
            }
            zwlr_virtual_pointer_v1::Request::Frame => {
                pointer_frame(state, resource);
            }
            zwlr_virtual_pointer_v1::Request::AxisSource { axis_source } => {
                pointer_axis_source(state, resource, axis_source);
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { axis, .. } => {
                pointer_axis_stop(state, resource, axis);
            }
            zwlr_virtual_pointer_v1::Request::AxisDiscrete {
                axis,
                value,
                discrete,
                ..
            } => {
                pointer_axis_discrete(state, resource, axis, value, discrete);
            }
            zwlr_virtual_pointer_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwlrVirtualPointerV1,
    ) {
        // Explicit destroy or client disconnect: the device is gone either
        // way, and anything it still holds must be released to whoever has
        // focus now -- a vanished remote pressing "down" forever is a stuck
        // button the user cannot lift.
        if let Some(device) = state.virtual_input.pointers.remove(resource) {
            release_pointer_buttons(state, device.held);
        }
    }
}

/// Per-keyboard user data. Empty, for the same reason as the pointer's.
struct VirtualKeyboardUserData;

impl Dispatch2<ZwpVirtualKeyboardV1, State> for VirtualKeyboardUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwpVirtualKeyboardV1,
        request: <ZwpVirtualKeyboardV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwp_virtual_keyboard_v1::Request::Keymap { format, fd, size } => {
                upload_keymap(state, resource, format, fd, size);
            }
            zwp_virtual_keyboard_v1::Request::Key {
                key,
                state: key_state,
                ..
            } => {
                virtual_key(state, resource, key, key_state);
            }
            zwp_virtual_keyboard_v1::Request::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                virtual_modifiers(
                    state,
                    resource,
                    mods_depressed,
                    mods_latched,
                    mods_locked,
                    group,
                );
            }
            zwp_virtual_keyboard_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwpVirtualKeyboardV1,
    ) {
        // Explicit destroy or client disconnect: the device is gone either
        // way, and anything it still holds must be released to whoever has
        // focus now -- a vanished remote pressing "down" forever is a stuck
        // key the user cannot lift. With binds off that is Smithay's
        // [`KeyboardHandle::release_source`], which forwards the releases
        // directly (nothing was ever intercepted, so there is nothing to
        // swallow). With binds on the releases go through the bind-aware
        // filter instead (`release_held_virtual_keys`): an intercepted
        // press's release is swallowed rather than forwarded as a lone
        // release the client never pressed, and no entry strands in
        // `virtual_suppressed`.
        if let Some(device) = state.virtual_input.keyboards.remove(resource) {
            if state.virtual_input_binds {
                release_held_virtual_keys(state, device.source, device.held);
            } else if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.release_source(state, device.source);
            }
        }
    }
}

/// Creates one virtual pointer, optionally mapped to `output`. The seat is
/// a suggestion the protocol lets the client make; scoot has exactly one,
/// so every pointer drives it. Called for both creation requests, which
/// differ only in whether an output was named.
fn create_pointer(
    state: &mut State,
    data_init: &mut DataInit<'_, State>,
    _seat: Option<WlSeat>,
    output: Option<WlOutput>,
    id: New<ZwlrVirtualPointerV1>,
) {
    let pointer: ZwlrVirtualPointerV1 = data_init.init(id, VirtualPointerUserData);
    state.virtual_input.pointers.insert(
        pointer,
        VirtualPointer {
            output,
            pending: PendingAxis::default(),
            held: 0,
        },
    );
}

/// Relative motion: no acceleration of its own, so the delta and the
/// unaccelerated delta are the same number (see `move_absolute`'s doc).
/// Dropped while locked, like every other virtual input event.
fn pointer_relative(state: &mut State, resource: &ZwlrVirtualPointerV1, dx: f64, dy: f64) {
    if state.session_lock.is_locked() {
        return;
    }
    if !state.virtual_input.pointers.contains_key(resource) {
        return;
    }
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    state.pointer_move_relative(dx, dy, dx, dy);
}

/// Absolute motion in the device's frame, mapped onto its output (or the
/// union) and clamped there. Dropped while locked.
fn pointer_absolute(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    x: u32,
    y: u32,
    x_extent: u32,
    y_extent: u32,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let Some(device) = state.virtual_input.pointers.get(resource) else {
        return;
    };
    let target = pointer_target(state, device.output.as_ref());
    let Some(target) = target else {
        return;
    };
    // A zero extent names only position zero: map it to the target's
    // origin rather than dividing by zero. Positions past a nonzero extent
    // clamp onto the target below instead of erroring -- outputs come and
    // go, and the clamp is always mappable.
    let fx = if x_extent == 0 {
        0.0
    } else {
        x.min(x_extent) as f64 / f64::from(x_extent)
    };
    let fy = if y_extent == 0 {
        0.0
    } else {
        y.min(y_extent) as f64 / f64::from(y_extent)
    };
    let (x, y) = (
        f64::from(target.loc.x) + fx * f64::from(target.size.w),
        f64::from(target.loc.y) + fy * f64::from(target.size.h),
    );
    let (x, y) = clamp_to_union(state, x, y);
    state.pointer_move(x, y);
}

/// The rectangle absolute motion maps onto: the device's bound output when
/// it names one this compositor still has, else the output union. `None`
/// only before any output exists.
fn pointer_target(state: &State, output: Option<&WlOutput>) -> Option<Rectangle<i32, Logical>> {
    if let Some(named) = output {
        if let Some(known) = state.outputs.iter().find(|known| known.owns(named)) {
            if let Some(geometry) = state.space.output_geometry(known) {
                return Some(geometry);
            }
        }
    }
    state.output_union()
}

/// Clamps onto the output union (bounding box; see
/// `clamp_to_output_union`'s doc for the dead-zone shape). Absolute motion
/// bypasses `pointer_move_relative`'s clamp, so this is where the output
/// edge holds.
fn clamp_to_union(state: &State, x: f64, y: f64) -> (f64, f64) {
    let Some(union) = state.output_union() else {
        return (0.0, 0.0);
    };
    let clamp = |value: f64, start: i32, extent: i32| {
        value.clamp(
            f64::from(start),
            f64::from(start + extent.saturating_sub(1).max(0)),
        )
    };
    (
        clamp(x, union.loc.x, union.size.w),
        clamp(y, union.loc.y, union.size.h),
    )
}

/// The Linux `BTN_*` code for a button, or `None` for one this compositor
/// has no name for. Dropped with a debug log rather than guessed at: the
/// five named buttons are every code wayvnc sends.
fn virtual_button(button: u32) -> Option<PointerButton> {
    match button {
        0x110 => Some(PointerButton::Left),
        0x111 => Some(PointerButton::Right),
        0x112 => Some(PointerButton::Middle),
        0x115 => Some(PointerButton::Back),
        0x116 => Some(PointerButton::Forward),
        _ => None,
    }
}

/// One bit per button in [`VirtualPointer::held`], indexed by
/// [`PointerButton`] discriminant. Exactly five buttons exist, so a `u32`
/// holds them all with room to spare and no allocation.
fn held_bit(button: PointerButton) -> u32 {
    1 << button as u32
}

/// A button press or release through [`State::pointer_button`] -- focus,
/// serials, popup dismissal and floating-drag settling exactly like a
/// physical click. Pressed is `== 1` (`wl_pointer.button_state`'s pressed;
/// any other value releases rather than sticking a button the client
/// mis-encoded). Dropped while locked.
fn pointer_button(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    button: u32,
    button_state: WEnum<wl_pointer::ButtonState>,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let Some(button) = virtual_button(button) else {
        tracing::debug!(
            button,
            "virtual pointer button with no mapping; dropping it"
        );
        return;
    };
    let pressed = matches!(button_state, WEnum::Value(wl_pointer::ButtonState::Pressed));
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    let bit = held_bit(button);
    if pressed {
        device.held |= bit;
    } else {
        device.held &= !bit;
    }
    state.pointer_button(button, pressed);
}

/// Releases every button in `held` (a disconnect, lock or VT sweep: the
/// device is gone or silenced, so whatever it held must go up). Each goes
/// through [`State::pointer_button`] like a physical release.
fn release_pointer_buttons(state: &mut State, held: u32) {
    for button in [
        PointerButton::Left,
        PointerButton::Right,
        PointerButton::Middle,
        PointerButton::Back,
        PointerButton::Forward,
    ] {
        if held & held_bit(button) != 0 {
            state.pointer_button(button, false);
        }
    }
}

/// Buffers one continuous axis value into the device's pending frame.
/// An unknown axis is the protocol's `invalid_axis` error: the client is
/// speaking a newer (or corrupt) enumeration, and guessing a direction
/// would scroll the wrong way.
fn pointer_axis(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    axis: WEnum<wl_pointer::Axis>,
    value: f64,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let horizontal = match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => true,
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => false,
        _ => {
            resource.post_error(
                zwlr_virtual_pointer_v1::Error::InvalidAxis,
                "axis is not a wl_pointer.axis value",
            );
            return;
        }
    };
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    if value.is_finite() {
        device.pending.push_axis(horizontal, value);
    }
}

/// Buffers one discrete step count. `value` (touchpad-coordinate units,
/// what `axis` carries per click) rides along when finite; `discrete`
/// becomes 120ths unless a `value120`-style overwrite already said it --
/// [`PendingAxis`] only knows `axis_discrete`, which is all this protocol
/// sends, so the overwrite never triggers here, and the comment stays with
/// the shared accumulator rather than this call site.
fn pointer_axis_discrete(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    axis: WEnum<wl_pointer::Axis>,
    value: f64,
    discrete: i32,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let horizontal = match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => true,
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => false,
        _ => {
            resource.post_error(
                zwlr_virtual_pointer_v1::Error::InvalidAxis,
                "axis is not a wl_pointer.axis value",
            );
            return;
        }
    };
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    if value.is_finite() {
        device.pending.push_axis(horizontal, value);
    }
    device.pending.push_discrete(horizontal, discrete);
}

/// Buffers one axis stop. Same error shape as [`pointer_axis`].
fn pointer_axis_stop(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    axis: WEnum<wl_pointer::Axis>,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let horizontal = match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => true,
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => false,
        _ => {
            resource.post_error(
                zwlr_virtual_pointer_v1::Error::InvalidAxis,
                "axis is not a wl_pointer.axis value",
            );
            return;
        }
    };
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    device.pending.push_stop(horizontal);
}

/// Records the scroll source for the pending frame. Same error shape, with
/// the protocol's own `invalid_axis_source` code.
fn pointer_axis_source(
    state: &mut State,
    resource: &ZwlrVirtualPointerV1,
    axis_source: WEnum<wl_pointer::AxisSource>,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let source = match axis_source {
        WEnum::Value(wl_pointer::AxisSource::Wheel) => AxisSource::Wheel,
        WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
        WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
        WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,
        _ => {
            resource.post_error(
                zwlr_virtual_pointer_v1::Error::InvalidAxisSource,
                "axis_source is not a wl_pointer.axis_source value",
            );
            return;
        }
    };
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    device.pending.push_source(source);
}

/// Ends one scroll sequence: whatever the device buffered since its last
/// frame goes out as a single scroll. Dropped while locked (like the events
/// that built it); an empty frame sends nothing.
fn pointer_frame(state: &mut State, resource: &ZwlrVirtualPointerV1) {
    if state.session_lock.is_locked() {
        return;
    }
    let Some(device) = state.virtual_input.pointers.get_mut(resource) else {
        return;
    };
    let mut pending = std::mem::take(&mut device.pending);
    let time = InputTime::from_millis(state.millis());
    if let Some(frame) = pending.finish(time) {
        state.emit_scroll(frame);
    }
}

/// Accepts one virtual keyboard's keymap upload. Only `xkb_v1` is defined;
/// anything else is ignored (the keymap stays unset, so the keys that
/// follow answer `no_keymap` -- the same shape Smithay's own manager gives
/// an unsupported format). An unreadable or unparsable fd is ignored the
/// same way, keeping whatever keymap the device already had: a broken
/// upload must not take a working keyboard offline.
fn upload_keymap(
    state: &mut State,
    resource: &ZwpVirtualKeyboardV1,
    format: u32,
    fd: OwnedFd,
    size: u32,
) {
    let Some(device) = state.virtual_input.keyboards.get_mut(resource) else {
        return;
    };
    if format != KeymapFormat::XkbV1 as u32 {
        tracing::debug!(
            format,
            "virtual keyboard keymap with unknown format; ignoring it"
        );
        return;
    }
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // SAFETY: mapping the client's own fd read-only, exactly as Smithay's
    // virtual-keyboard manager does. A short or empty fd fails the map (or
    // the compile) and keeps the old keymap -- see above.
    let keymap = match unsafe {
        xkb::Keymap::new_from_fd(
            &context,
            fd,
            size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    } {
        Ok(Some(keymap)) => keymap,
        Ok(None) => {
            tracing::debug!("virtual keyboard keymap did not compile; keeping the old one");
            return;
        }
        Err(error) => {
            tracing::debug!(
                ?error,
                "virtual keyboard keymap could not be mapped; keeping the old one"
            );
            return;
        }
    };
    device.keymap = Some(VirtualKeymap {
        state: xkb::State::new(&keymap),
    });
}

/// One virtual key press or release, translated by keysym into the seat
/// layout and delivered through the seat (see the module doc). `1` presses;
/// any other state value releases rather than sticking a key the client
/// mis-encoded. Dropped while locked; without an uploaded keymap this is
/// the protocol's `no_keymap` error; a keysym the seat layout cannot type
/// is dropped with a debug log.
///
/// With `[virtual_input] binds` on, a press matching the bind table by its
/// translated seat keysym runs the bind (see [`virtual_filter`]) and is
/// intercepted rather than forwarded; its release is intercepted too, via
/// `virtual_suppressed`. Fire-once, never repeat, never while locked.
fn virtual_key(state: &mut State, resource: &ZwpVirtualKeyboardV1, key: u32, key_state: u32) {
    if state.session_lock.is_locked() {
        return;
    }
    // The press-time seat code for this virtual position, when this device
    // already holds it: a release must free what its press sent, not what
    // today's device mask would translate the position to (see `held`).
    let held_code = state
        .virtual_input
        .keyboards
        .get(resource)
        .and_then(|device| {
            device
                .held
                .iter()
                .find(|(held_key, _)| *held_key == key)
                .map(|(_, code)| *code)
        });
    let keysym = match held_code {
        // A release of a held key needs no keymap and no translation: the
        // press already decided what this key is.
        Some(_) => None,
        None => {
            let Some(device) = state.virtual_input.keyboards.get(resource) else {
                return;
            };
            let Some(virtual_map) = device.keymap.as_ref() else {
                resource.post_error(
                    zwp_virtual_keyboard_v1::Error::NoKeymap,
                    "`key` sent before any keymap",
                );
                return;
            };
            // Wayland keycodes are evdev codes; xkb numbers from 8. A wrapped
            // addition would translate a nonsense position into a real keysym, so
            // the overflow refuses rather than mistypes.
            let Some(code) = key.checked_add(8).map(Keycode::new) else {
                tracing::debug!(
                    key,
                    "virtual key code overflows the xkb numbering; dropping it"
                );
                return;
            };
            let keysym = virtual_map.state.key_get_one_sym(code);
            if keysym.raw() == 0 {
                tracing::debug!(key, "virtual key with no keysym in its keymap; dropping it");
                return;
            }
            Some(keysym)
        }
    };
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let seat_code = match held_code {
        Some(code) => code,
        None => {
            let keysym = keysym.expect("a translated keysym when nothing is held");
            match keyboard.keycode_for_keysym(keysym) {
                Some(code) => code,
                None => {
                    tracing::debug!(
                        keysym = xkb::keysym_get_name(keysym),
                        "virtual keysym has no key in the seat layout; dropping it"
                    );
                    return;
                }
            }
        }
    };
    let pressed = key_state == 1;
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    let time = InputTime::from_millis(state.millis());
    state.announce_activity();
    // The device's source for the filter's suppression bookkeeping. Read
    // before `input_from_source` takes `&mut State`; the borrow ends here.
    let source = state
        .virtual_input
        .keyboards
        .get(resource)
        .map(|device| device.source);
    let Some(source) = source else {
        return;
    };
    keyboard.input_from_source(
        source,
        state,
        seat_code,
        if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        },
        serial,
        time,
        |data, mods, handle| virtual_filter(data, pressed, seat_code, mods, handle),
    );
    // Mirror Smithay's per-source holders for this device (which records
    // the press even when absorbed as a non-transition): the binds-on
    // teardown synthesizes releases for exactly this set. Tracked only
    // with binds on -- with it off the teardown is `release_source`,
    // which needs no mirror, so the off path touches no state past the
    // filter's one bool (see the module doc's inertness note).
    if state.virtual_input_binds {
        if let Some(device) = state.virtual_input.keyboards.get_mut(resource) {
            if pressed {
                if !device.held.iter().any(|(held_key, _)| *held_key == key) {
                    device.held.push((key, seat_code));
                }
            } else {
                device.held.retain(|(held_key, _)| *held_key != key);
            }
        }
    }
    state.check_keyboard_layout();
}

/// The `input_from_source` filter for virtual keys: forward-only with
/// `[virtual_input] binds` off (one bool, no table lookup), bind-aware on.
///
/// Presses match by the seat keysym the filter reads live
/// (`raw_syms().first()`, the unshifted symbol -- the same projection
/// `input::key` matches on) with the seat modifiers Smithay derived for
/// this key, so a remote layout that moved the key still names the seat's
/// bind. A match runs through `act_bind` and intercepts; anything else --
/// flag off, no match, or locked -- forwards. (The locked forward is
/// belt-and-braces: `virtual_key` drops presses while locked before this
/// filter is ever built, and teardown only ever synthesizes releases.)
/// Releases never consult the table or the flag: an intercepted press's
/// release is swallowed by `virtual_suppressed`, whatever gained focus in
/// between, and every other release is forwarded.
///
/// `pressed` travels by closure capture (Smithay's filter signature carries
/// no key state -- the physical path does exactly the same): the two
/// halves never share routing. Deliberately no bind repeat: a flagged bind
/// re-fires while a physical key is held, but a virtual hold has no repeat
/// lifecycle the compositor owns (see the module doc), so a virtual bind
/// fires exactly once. And deliberately no
/// `held_keys`/`suppressed_keys`/`interaction_serials` touches: those
/// mirror the physical `MAIN` source, which this path must leave exact.
fn virtual_filter(
    data: &mut State,
    pressed: bool,
    seat_code: Keycode,
    mods: &ModifiersState,
    handle: KeysymHandle<'_>,
) -> FilterResult<()> {
    if !pressed {
        // Routed by what the press decided, never by the table mid-hold,
        // and never gated on the lock (a sweep release must swallow even
        // under lock) or the flag (restart-only: it cannot have flipped
        // mid-hold, and swallowing must not depend on it if it ever could).
        if data.virtual_suppressed.remove(&seat_code) {
            return FilterResult::Intercept(());
        }
        return FilterResult::Forward;
    }
    // The off path: one bool, no table lookup, no allocation, no state
    // touched. The physical per-keypress path never reaches this filter at
    // all, so this branch is the whole of the default-off cost on virtual
    // keys -- and zero on physical ones.
    if !data.virtual_input_binds {
        return FilterResult::Forward;
    }
    let Some(&keysym) = handle.raw_syms().first() else {
        return FilterResult::Forward;
    };
    let Some((bound, flags)) = data.keybindings.match_key(keysym, mods.into()) else {
        return FilterResult::Forward;
    };
    // Absolute, like the drop in `virtual_key`: a virtual key never runs a
    // bind while locked -- not even an `allow_when_locked` spawn. Forward
    // rather than swallow (the physical path's rule for disallowed binds),
    // though this is unreachable while the drop above stands.
    if data.session_lock.is_locked() {
        return FilterResult::Forward;
    }
    // Remember this keycode was intercepted so the matching release is
    // intercepted too, rather than forwarded to whatever gained focus in
    // between (e.g. after this action closes the current focus) as a
    // spurious lone release it never pressed. Keyed by keycode alone --
    // Smithay runs this filter only on transitions (the first source to
    // press, the last to release), so two virtual keyboards holding one
    // key share the one entry and the bind still fires exactly once.
    data.virtual_suppressed.insert(seat_code);
    match bound {
        Bound::Action(action) => {
            data.act_bind(action, flags.allow_when_locked);
        }
        Bound::ChangeVt(vt) => {
            let _ = data.change_vt(vt);
        }
    }
    FilterResult::Intercept(())
}

/// One virtual `modifiers` request: updates the device's own xkb mask (and
/// group) so the keysyms the keys that follow translate against stay right.
/// The mask is *not* applied to the seat -- the seat learns modifiers from
/// the translated modifier key events themselves, which is what keeps the
/// two layouts' different modifier wirings from leaking into each other
/// (see the module doc). Without an uploaded keymap this is `no_keymap`,
/// like [`virtual_key`].
fn virtual_modifiers(
    state: &mut State,
    resource: &ZwpVirtualKeyboardV1,
    mods_depressed: u32,
    mods_latched: u32,
    mods_locked: u32,
    group: u32,
) {
    if state.session_lock.is_locked() {
        return;
    }
    let Some(device) = state.virtual_input.keyboards.get_mut(resource) else {
        return;
    };
    let Some(virtual_map) = device.keymap.as_mut() else {
        resource.post_error(
            zwp_virtual_keyboard_v1::Error::NoKeymap,
            "`modifiers` sent before any keymap",
        );
        return;
    };
    virtual_map
        .state
        .update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
}

/// Releases `held` (press-time seat codes) for `source` through the
/// bind-aware filter: an intercepted press's release is swallowed via
/// `virtual_suppressed`, every other release is forwarded to whoever has
/// focus -- the same destination `release_source` would have used, but
/// without its filter bypass. Runs even under lock (releases are never
/// lock-gated); a missing seat keyboard drops the keys and scrubs the
/// suppression entries so nothing strands.
fn release_held_virtual_keys(state: &mut State, source: KeyboardSource, held: Vec<(u32, Keycode)>) {
    if held.is_empty() {
        return;
    }
    let Some(keyboard) = state.seat.get_keyboard() else {
        for (_, code) in held {
            state.virtual_suppressed.remove(&code);
        }
        return;
    };
    for (_, seat_code) in held {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(state.millis());
        keyboard.input_from_source(
            source,
            state,
            seat_code,
            KeyState::Released,
            serial,
            time,
            |data, mods, handle| virtual_filter(data, false, seat_code, mods, handle),
        );
    }
}

impl State {
    /// Releases everything every virtual device holds: each keyboard's held
    /// keys (their releases go up on whoever has focus) and each pointer's
    /// held buttons. Pending scroll sequences are dropped rather
    /// than delivered after the interruption: a half-sent wheel gesture from
    /// before the lock (or the switch away) is stale motion, not a scroll
    /// the user is still making. Called when the session locks (first, so
    /// the releases reach whoever held them), when the VT switches away,
    /// and -- per device -- when a device is destroyed (see the `destroyed`
    /// impls above, which release one device through the same choice).
    ///
    /// With binds off that is Smithay's `release_source`, which forwards
    /// the releases directly (nothing was ever intercepted). With binds on
    /// the releases go through the bind-aware filter instead
    /// (`release_held_virtual_keys`): an intercepted press's release is
    /// swallowed rather than forwarded as a lone release, and the sweep
    /// runs even under lock (releases are never lock-gated -- only presses
    /// are dropped).
    pub(super) fn release_virtual_input(&mut self) {
        if self.virtual_input_binds {
            let held: Vec<(KeyboardSource, Vec<(u32, Keycode)>)> = self
                .virtual_input
                .keyboards
                .values_mut()
                .map(|device| (device.source, std::mem::take(&mut device.held)))
                .collect();
            for (source, keys) in held {
                release_held_virtual_keys(self, source, keys);
            }
        } else {
            let sources: Vec<KeyboardSource> = self
                .virtual_input
                .keyboards
                .values()
                .map(|device| device.source)
                .collect();
            if let Some(keyboard) = self.seat.get_keyboard() {
                for source in sources {
                    keyboard.release_source(self, source);
                }
            }
        }
        let held: Vec<u32> = self
            .virtual_input
            .pointers
            .values_mut()
            .map(|device| {
                // Stale scroll first (see the method doc), then the buttons.
                device.pending = PendingAxis::default();
                std::mem::replace(&mut device.held, 0)
            })
            .collect();
        for mask in held {
            release_pointer_buttons(self, mask);
        }
    }
}
