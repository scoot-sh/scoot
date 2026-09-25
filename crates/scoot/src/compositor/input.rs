//! Injected input: what an agent sends in place of a keyboard and mouse.

use scoot_core::Action;
use scoot_ipc::{KeyCombo, Modifier, PointerButton};
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, KeyboardHandle, Keycode, Keysym, xkb};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, MotionEvent, PointerHandle, RelativeMotionEvent,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial};
use smithay::wayland::pointer_constraints::with_pointer_constraint;
use smithay::wayland::seat::WaylandFocus;

use super::State;
use super::keybindings::Bound;
use super::layer_shell;
use super::output_scale::logical_size;
use super::relative_pointer::{AbsoluteTarget, absolute_target};
use super::tty::VtSwitchOutcome;
use modifiers::{HeldKeys, KeyPlan, NamedKey, Untypable};

mod compose;
pub(super) mod interaction;
mod modifiers;
#[cfg(test)]
mod tests;

// Linux input event codes, which is what Wayland carries.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Every injected-input entry point needs the seat's keyboard, both to send
/// keys and to read the keymap. `State::new` always adds one and nothing
/// removes it, so this is a guard rather than an expected outcome -- but it
/// is a distinct answer from anything the *keymap* could say, and saying so
/// is what keeps "this layout has no `é`" from also meaning "there is no
/// keyboard".
const NO_KEYBOARD: &str = "this seat has no keyboard";

/// What handling one key press/release actually did. `Default` gives the
/// right answer (`intercepted: false, vt_switch: None`) for the
/// no-keyboard-yet early return in [`State::key`] and for `keyboard.input`'s
/// own `None` case. Per the pinned Smithay rev's
/// `KeyboardHandle::input_from_source` (`src/input/keyboard/mod.rs`), that
/// `None` covers two things, neither of which is about focus: this exact
/// keycode transition already being absorbed by another input source
/// holding it (the `!is_transition` check -- avoids double-running the
/// filter and forwarding a duplicate), or the filter closure below
/// returning `FilterResult::Forward` (forwarded to the focused client via
/// `input_forward`, nothing intercepted). Either way it's "nothing
/// happened," not "something happened and nothing switched."
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct KeyOutcome {
    /// Whether this press or release was intercepted by a keybinding rather
    /// than forwarded to the focused client.
    pub intercepted: bool,
    /// `Some` only when this exact call is the one that invoked
    /// `change_vt` (the `Bound::ChangeVt` arm below, on a press) -- `None`
    /// for a plain forwarded key, an `Action` binding, or any release, all
    /// of which never touch VT switching.
    pub vt_switch: Option<VtSwitchOutcome>,
}

impl State {
    pub fn pointer_move(&mut self, x: f64, y: f64) {
        self.announce_activity();
        self.pointer_move_quietly(x, y);
    }

    /// [`State::pointer_move`] without the activity announcement: the
    /// compositor re-running its own hit test is not a user at the
    /// machine, and announcing it would restart every idle timer on
    /// each lock transition. The only caller is
    /// [`State::refresh_pointer_focus`]; every real motion source --
    /// libinput (`pointer_move_relative`), the host (`nested_dispatch`),
    /// IPC injection -- goes through `pointer_move`.
    fn pointer_move_quietly(&mut self, x: f64, y: f64) {
        self.move_absolute(x, y, None);
    }

    /// The absolute-motion core every pointer source reaches: `pointer_move`
    /// (IPC, `--nested`, absolute tablets), `pointer_move_relative` (`--tty`
    /// libinput) after clamping, and the two quiet re-derivations above.
    ///
    /// `device` is `Some((delta, unaccel))` only on the libinput path, which
    /// is the one source that reports relative device motion rather than an
    /// absolute position: the accelerated pair moves the absolute position
    /// (after clamping, by the caller), the pre-accel pair is what the
    /// relative event reports as unaccelerated. `None` derives both from the
    /// position change itself -- every absolute source applies no
    /// acceleration of its own, so delta and unaccelerated delta are the
    /// same number there (see `relative_pointer.rs`).
    ///
    /// Relative motion is emitted against pre-move focus on every focused
    /// move with a nonzero delta, *before* the absolute motion (Smithay
    /// routes by the seat's current focus, so this order is what credits the
    /// surface the pointer is leaving; see `relative_pointer.rs`). It is --
    /// this is the point of the protocol -- unclipped: a move the output
    /// edge or an active lock/confinement trims still reports the full
    /// vector. A locked pointer moves nothing absolute at all
    /// ([`AbsoluteTarget::Held`]); the event and the frame still go out.
    ///
    /// Cost on the hot path, measured before/after on the dev VM (200k
    /// `pointer_move` events x 5 reps, temporary bench since removed).
    /// Debug: unfocused 3129-3426 ns/event after vs 3415-3623 before --
    /// ranges overlapping, no measurable regression; focused with no
    /// relative pointers bound 10688-11501 vs 8555-9090 before. Release
    /// (the profile that ships): unfocused 336-388 after vs 354-362
    /// before -- overlapping, noise; focused 950-998 vs 757-823 before,
    /// a ~190ns residual: one `current_location` read, one
    /// constraint-map lookup and Smithay's empty-list lock per event.
    /// At a real 1000Hz device rate that residual is ~190us/s -- about
    /// 1.2% of a single 16ms frame per second, 0.02% of a core -- and a
    /// session with a live relative pointer pays per-object socket writes
    /// beside it either way. The baseline everywhere is the hit test plus
    /// idle announce plus socket work, not this check (an unfocused move
    /// never reads the location and never touches the constraint map).
    fn move_absolute(
        &mut self,
        x: f64,
        y: f64,
        device: Option<(Point<f64, Logical>, Point<f64, Logical>)>,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let location = Point::<f64, Logical>::from((x, y));
        let under = self.surface_under(location);
        // Deliberately not recorded in `interaction_serials` (unlike the
        // button and key serials below): motion is continuous and passive.
        // libinput reports it at 500-1000Hz just from a hand resting on a
        // desk, and `refresh_pointer_focus` above synthesizes it with no
        // user involvement at all -- treating that as "the user asked for
        // this" would hand every client a permanent, self-refreshing
        // activation serial and defeat the gate in `activation.rs`.
        let serial = SERIAL_COUNTER.next_serial();
        // The one exception, for the popup-grab gate (`popup.rs`): a motion
        // that actually *changes* pointer focus delivers an `enter` carrying
        // this serial, which is what a toolkit passes to `xdg_popup.grab`
        // when a menu was opened by hovering. Recorded under the entered
        // client, like everything else in `interaction_serials` -- and only
        // then: every other motion delivers nothing new, and crediting one
        // would file a serial its client never saw under its name. Recorded
        // per arm below rather than here, because a move a lock or
        // confinement holds delivers no `enter` at all: recording one would
        // file a serial under a client that never received it, the exact
        // hole the client half of each entry exists to close.
        //
        // Two guards, each load-bearing (see `record_pointer_enter`):
        // - the focus must really move (`current_focus` read before `motion`
        //   below updates it). A redundant motion mints a serial but sends
        //   no `enter`.
        // - no grab may hold the pointer. Under one the recipient is the
        //   grab's own logic, not `under` -- crediting `under` would file an
        //   entry for a client that never received it.
        //
        // Costs, on the common no-change path that real motion takes at
        // 500-1000Hz: one seat lock plus a surface-handle clone
        // (`current_focus`), nothing else. The second lock (`is_grabbed`)
        // runs only when focus actually moved, and the backend client lookup
        // (`client_of`) only when something was actually entered -- both
        // rare next to the hit test above and the client socket write below,
        // which this path already pays per event. Measured on the dev VM, a
        // 200k no-change `pointer_move` stream, before/after: 4799-5090 vs
        // 4575-4902 ns/event across five reps each -- ranges overlapping, no
        // measurable regression; the ~4.7us baseline is hit test, idle
        // announce and socket work, not this check.
        let time = InputTime::from_millis(self.millis());
        // The move the absolute position actually makes, resolved against
        // the pre-move focus surface's lock/confinement; the relative event
        // below still carries the full vector either way. Location and
        // deltas are read only with a focus to credit or a constraint to
        // resolve -- both need one -- so an unfocused move skips straight
        // to delivery with one `current_focus` read as its whole added
        // cost (see the measured ranges in this function's doc above).
        let focus = pointer.current_focus();
        let (target, delta, unaccel) = match &focus {
            Some(_) => {
                let from = pointer.current_location();
                let derived = location - from;
                let (delta, unaccel) = device.unwrap_or((derived, derived));
                let target =
                    absolute_target(self, &pointer, focus.as_ref(), from, location, &under);
                (target, delta, unaccel)
            }
            None => (
                AbsoluteTarget::Free,
                Point::from((0.0, 0.0)),
                Point::from((0.0, 0.0)),
            ),
        };
        // Focus-gated, not lock-gated (see `relative_pointer.rs`): whoever
        // holds pointer focus gets the deltas, locked or not, and nobody
        // else does. Zero deltas stay silent -- a focus re-derivation at a
        // standstill is not motion, and the wire has enough of it already.
        if focus.is_some() && (delta.x != 0.0 || delta.y != 0.0) {
            pointer.relative_motion(
                self,
                under.clone(),
                &RelativeMotionEvent {
                    delta,
                    delta_unaccel: unaccel,
                    time,
                },
            );
        }
        match target {
            AbsoluteTarget::Free => {
                if self.record_pointer_enter(&pointer, &focus, &under, serial) {
                    // Focus just landed on a new surface: engage a
                    // still-inactive constraint waiting on it (see below).
                    // Gated on the enter, not run per move: activation is a
                    // creation/arrival event, and a redundant move has no
                    // new surface to arm.
                    self.engage_pending_constraint(&pointer, &under, location);
                }
                self.move_pointer_to(&pointer, under, location, serial, time);
            }
            AbsoluteTarget::Clamped { point, under } => {
                self.record_pointer_enter(&pointer, &focus, &Some(under.clone()), serial);
                self.move_pointer_to(&pointer, Some(under), point, serial, time);
            }
            AbsoluteTarget::Held => {
                // Locked (or confined off its surface): relative went out
                // above, absolute moves nothing, and the cursor bitmap sits
                // where it was, so there is nothing to redraw either. The
                // frame still goes: it is what releases the relative event.
                // No `enter` is recorded either: nothing was delivered.
                pointer.frame(self);
            }
        }
        // A floating window dragged onto another output, or a drag that
        // ended at this motion, asked for a full `apply()` it could not run
        // inside the pointer lock (see `floating/grab.rs`). One `bool` test
        // on every other motion.
        self.settle_floating_grab();
    }

    /// Records a focus-changing motion's `enter` serial for the popup-grab
    /// gate, or records nothing when the motion delivers no `enter`.
    ///
    /// Split out of [`State::move_absolute`] so each resolution arm records
    /// against the surface its motion is actually delivered with -- not, in
    /// particular, the target hit test of a move confinement clamped
    /// elsewhere, and never anything on the held path, which delivers no
    /// motion at all. Answers whether focus moved, so the free arm knows
    /// whether a pending constraint wants engaging.
    ///
    /// `focus` is the pre-move focus `move_absolute` already read, passed
    /// in rather than re-read: nothing between that read and this call can
    /// move seat focus (`current_location` and `absolute_target` are
    /// read-only, and `relative_motion` routes by focus without setting
    /// it), so the re-read the reviewer flagged observed exactly this
    /// value. What remains read here is only the grab check, which has no
    /// cheaper source.
    fn record_pointer_enter(
        &mut self,
        pointer: &PointerHandle<Self>,
        focus: &Option<WlSurface>,
        under: &Option<(WlSurface, Point<f64, Logical>)>,
        serial: Serial,
    ) -> bool {
        let entered = Self::pointer_entered(pointer, focus, under);
        if entered
            && let Some((surface, _)) = &under
            && let Some(client) = self.client_of(surface)
        {
            self.interaction_serials.record_focus(serial, client);
        }
        entered
    }

    /// Activates a still-inactive lock or confinement on a newly entered
    /// surface -- the late half of
    /// [`new_constraint`](super::relative_pointer::PointerConstraintsHandler),
    /// which only fires at creation time.
    ///
    /// Without this, a lock taken before first focus (a game arming its
    /// mouse mode at startup, ahead of any pointer motion) would sit
    /// inactive forever: Smithay reports `locked`/`confined` only from an
    /// explicit `activate`, and nothing else ever calls it. The region gate
    /// is anvil's: a constraint whose region does not contain the arrival
    /// point stays disarmed. Called only on focus-changing moves (see the
    /// free arm above), so the steady-state motion path never pays for it.
    fn engage_pending_constraint(
        &mut self,
        pointer: &PointerHandle<Self>,
        under: &Option<(WlSurface, Point<f64, Logical>)>,
        location: Point<f64, Logical>,
    ) {
        let Some((surface, origin)) = under else {
            return;
        };
        with_pointer_constraint(surface, pointer, |constraint| {
            let Some(constraint) = constraint else {
                return;
            };
            if constraint.is_active() {
                return;
            }
            if constraint
                .region()
                .is_none_or(|region| region.contains((location - *origin).to_i32_round()))
            {
                constraint.activate();
            }
        });
    }

    /// Delivers one absolute motion and its frame, redrawing the cursor on
    /// `--tty` (the only backend that draws one) when the pointer moved.
    ///
    /// Split out of [`State::move_absolute`] so the free and clamped arms
    /// share the delivery: both move the pointer, both end the frame, and
    /// only they redraw. The held arm does neither (see above).
    fn move_pointer_to(
        &mut self,
        pointer: &PointerHandle<Self>,
        under: Option<(WlSurface, Point<f64, Logical>)>,
        location: Point<f64, Logical>,
        serial: Serial,
        time: InputTime,
    ) {
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial,
                time,
            },
        );
        pointer.frame(self);
        // Only `--tty` draws a visible cursor (see `cursor.rs`'s module
        // doc), so only it needs a redraw on plain motion -- headless and
        // nested have nothing on screen that changes when the pointer moves
        // without also pressing/scrolling/committing, and marking them
        // dirty here would cost a real render for no visible effect. What
        // they do have is capture sessions that asked for the pointer, which
        // `cursor_changed` keeps current without a render.
        // libinput can report motion at 500-1000Hz while the display flips
        // at ~60Hz; this only marks the frame dirty (or arms the tick);
        // `render()` still runs at most once per frame tick, not once per
        // event.
        self.cursor_changed();
    }

    /// Whether this motion will deliver a pointer `enter` to a client
    /// surface: focus really moves somewhere, and no grab holds the pointer.
    ///
    /// Under a grab the recipient is the grab's own logic, not `under` -- a
    /// popup grab confines the pointer to its own tree, an implicit button
    /// grab confines it to the pressed surface -- so `under` names a client
    /// that will never see this serial. `focus` is read by the caller
    /// before [`PointerHandle::motion`] runs: afterwards the seat's focus
    /// already names `under` and the move is unobservable.
    fn pointer_entered(
        pointer: &PointerHandle<Self>,
        focus: &Option<WlSurface>,
        under: &Option<(WlSurface, Point<f64, Logical>)>,
    ) -> bool {
        if focus.as_ref() == under.as_ref().map(|(surface, _)| surface) {
            return false;
        }
        !pointer.is_grabbed()
    }

    /// Centres the pointer on the output, once, at startup.
    ///
    /// Called from `headless::init_named` -- the one init point every
    /// backend (`--headless`, `--nested`, `--tty`) reaches -- so the cursor
    /// does not sit wedged in the top-left corner until the first motion.
    /// Reads the *logical* extent off the output itself (the same rectangle
    /// the core and the `Space` lay out in, and the space the pointer lives
    /// in), not a hardcoded rect, so a scale other than 1 -- or a future
    /// second output -- cannot silently misplace it. No output yet (the
    /// `None` arm, unreachable once `init_named` has run) centres on
    /// nothing, i.e. the origin Smithay starts at.
    ///
    /// Goes through [`State::pointer_move_quietly`], never the announcing
    /// path: startup placement is not a user at the machine, so it must not
    /// reset the idle timers, and with no client under it yet it derives no
    /// focus and mints no interaction serial. Deliberately *not* repeated
    /// on resize or VT-switch reactivation -- both leave the pointer where
    /// the user left it (`resize_output` never touches it, and neither does
    /// `session_event`'s reactivation arm).
    pub(super) fn place_pointer_at_output_centre(&mut self) {
        let (width, height) = self.outputs.primary().map(logical_size).unwrap_or((0, 0));
        self.pointer_move_quietly(f64::from(width) / 2.0, f64::from(height) / 2.0);
    }

    /// Re-runs the hit test where the pointer already is, so pointer focus
    /// follows a change in *what is on screen* rather than waiting for the
    /// user to move the mouse.
    ///
    /// Called at every session-lock transition, and that is not a nicety:
    /// `wl_pointer.button` and `wl_pointer.axis` go to whatever surface the
    /// pointer last *entered*, never to whatever is under it now. Without
    /// this, the first click after a lock would still land in the window
    /// underneath -- the hit test in [`State::surface_under`] would never be
    /// consulted, because nothing moved.
    pub(super) fn refresh_pointer_focus(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let location = pointer.current_location();
        self.pointer_move_quietly(location.x, location.y);
    }

    /// Moves the pointer by a relative delta, clamped to the union of every
    /// output's bounds (see [`State::clamp_to_output_union`]). `--tty`'s only
    /// source of pointer motion: unlike
    /// nested's host-forwarded motion (already absolute) or IPC's
    /// `pointer move X Y`, libinput reports relative dx/dy for a plain
    /// mouse, with no absolute position of its own -- this is what turns
    /// that into the same absolute `pointer_move` every other input source
    /// already uses. Kept backend-neutral like every other method here:
    /// nothing about it is `--tty`-specific, it just happens to be the one
    /// caller that needs it today.
    ///
    /// Two pairs, not one: `(dx, dy)` is the accelerated delta libinput
    /// reports (what the absolute position moves by, after clamping), and
    /// `(dx_unaccel, dy_unaccel)` is the pre-acceleration device delta --
    /// what `zwp_relative_pointer_v1.relative_motion` reports as the
    /// unaccelerated vector (see `relative_pointer.rs`). Passing the
    /// accelerated pair twice would label accelerated motion unaccelerated,
    /// which is exactly the misreport the protocol exists to let clients
    /// avoid.
    pub fn pointer_move_relative(&mut self, dx: f64, dy: f64, dx_unaccel: f64, dy_unaccel: f64) {
        self.announce_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let current = pointer.current_location();
        let (x, y) = self.clamp_to_output_union(current.x + dx, current.y + dy);
        // The absolute core, not `pointer_move`: activity was announced
        // above, and the relative deltas here are the raw device pairs, not
        // the clamped position change -- the relative event reports the
        // unclipped vector even when the absolute position stops at the
        // output edge (see `move_absolute`).
        self.move_absolute(
            x,
            y,
            Some((Point::from((dx, dy)), Point::from((dx_unaccel, dy_unaccel)))),
        );
    }

    pub fn pointer_button(&mut self, button: PointerButton, pressed: bool) {
        self.announce_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if pressed {
            self.focus_under_pointer();
        }
        let serial = SERIAL_COUNTER.next_serial();
        // After the focus change (which may `apply()`, outside any pointer
        // lock) and before the press is delivered: a modifier press on a
        // floating window starts dragging it, and the grab then swallows
        // this press -- which also keeps it out of `interaction_serials`
        // below, the grab having cleared pointer focus. See
        // `floating/grab.rs`.
        if pressed {
            self.begin_modifier_drag(&pointer, button, serial);
        }
        // Recorded against whoever `pointer.button` below is about to deliver
        // this to -- the surface the pointer last *entered*, which is what
        // the pointer's own focus is, not what is under it now and not
        // whatever `focus_under_pointer` just gave the keyboard to.
        //
        // Both states, not just the press: which of the two a client mints an
        // activation token from is its own choice (a GTK button activates on
        // release), and a tracker that only knew presses would refuse the
        // legitimate half of that. See `interaction.rs`.
        if let Some(client) = pointer
            .current_focus()
            .and_then(|surface| self.client_of(&surface))
        {
            self.interaction_serials.record(serial, client);
        }
        let time = InputTime::from_millis(self.millis());
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        pointer.button(
            self,
            &ButtonEvent {
                button: button_code(button),
                state,
                serial,
                time,
            },
        );
        pointer.frame(self);
        // A click outside an open menu is what dismisses it (Smithay's
        // `PopupPointerGrab` does that inside the call above), and it leaves
        // the keyboard on the popup's root rather than wherever scoot's own
        // policy would put it. Settling here rather than waiting for the
        // client's follow-up traffic keeps the two in step within the same
        // event. One `Option` check when no menu is open.
        self.settle_popup_grab();
        // A release (or a second press) that ended a floating window's drag
        // did it inside the pointer lock; the arrangement catches up here.
        self.settle_floating_grab();
    }

    pub fn scroll(&mut self, dx: f64, dy: f64) {
        self.announce_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let mut frame =
            AxisFrame::new(InputTime::from_millis(self.millis())).source(AxisSource::Wheel);
        if dx != 0.0 {
            frame = frame.value(Axis::Horizontal, dx);
        }
        if dy != 0.0 {
            frame = frame.value(Axis::Vertical, dy);
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Presses and releases a combination, holding its modifiers around it.
    ///
    /// *Exactly* the combination named, and nothing else: the key pressed is
    /// the one carrying `combo.key`'s keysym with nothing held, and the only
    /// modifiers held are the ones `combo` lists. A name this layout only
    /// carries further up (`exclam`, `at`, `A`) is refused rather than
    /// pressed, because the key carrying it types a *different* character
    /// when pressed bare -- `scoot msg key exclam` typed `1` for as long as
    /// it resolved names the way Smithay's `keycode_for_keysym` does, which
    /// takes the lowest keycode carrying a keysym at any level. Working out
    /// which modifiers a character needs is [`State::type_text`]'s job; this
    /// one does what it is told.
    ///
    /// If `combo` matches a keybinding, `key` runs it exactly as a real
    /// keypress would -- this doesn't bypass keybindings the way
    /// `Request::Action` does. That's deliberate: an agent using `press` is
    /// asking for input-level fidelity, and `Request::Action` already exists
    /// as the direct, binding-independent way to invoke a window-management
    /// action.
    ///
    /// Returns whichever [`VtSwitchOutcome`] any press in this sequence
    /// produced, if any. In practice only the *main* key's press can ever
    /// carry one: every `ChangeVt` binding is keyed on an `F1`..`F12` keysym
    /// (see `Keybindings::vt_switch_bindings`, the only place that
    /// constructs one -- `tty::init` only applies it),
    /// never on a modifier's own keysym (whichever hand it is on), so
    /// pressing one of this combo's modifiers on its own --
    /// the loop below -- structurally cannot match one, no matter what's
    /// configured in `[binds]`. Accumulated across *every* press below
    /// (via `Option::or`, so the first hit wins) rather than read from just
    /// the main key's call, so this stays correct even if that guarantee
    /// ever stopped holding, instead of silently depending on it. `ipc.rs`'s
    /// `Request::Key` handler uses the result to warn a caller whose only
    /// input/output is this IPC connection when a switch-away request just
    /// went out, since it may have just cost that connection the one
    /// channel that could switch the session back.
    pub fn press(&mut self, combo: &KeyCombo) -> Result<Option<VtSwitchOutcome>, String> {
        let (code, held) = self.resolve_combo(combo)?;
        let mut vt_switch = None;
        for &modifier in held.as_slice() {
            vt_switch = vt_switch.or(self.key(modifier, KeyState::Pressed).vt_switch);
        }
        vt_switch = vt_switch.or(self.key(code, KeyState::Pressed).vt_switch);
        self.key(code, KeyState::Released);
        for &modifier in held.as_slice().iter().rev() {
            self.key(modifier, KeyState::Released);
        }
        Ok(vt_switch)
    }

    /// The key [`State::press`] will press for `combo`, and the modifier
    /// keys to hold around it -- or why this layout can press neither.
    ///
    /// Resolved in one pass, before anything is pressed: these are pure
    /// lookups against the static keymap (no dependency on what's currently
    /// held), so doing it up front means a name this layout can't press is
    /// rejected before any key state changes, rather than after some
    /// modifiers are already down. Otherwise a failure partway through would
    /// leave a modifier (e.g. Super) permanently held at the seat level,
    /// silently corrupting every keypress after it.
    fn resolve_combo(&mut self, combo: &KeyCombo) -> Result<(Keycode, HeldKeys), String> {
        let keysym =
            keysym_named(&combo.key).ok_or_else(|| format!("unknown key `{}`", combo.key))?;
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| NO_KEYBOARD.to_owned())?;
        self.with_keymap(&keyboard, |keymap, layout| {
            let code = unmodified_key(keymap, layout, keysym, &combo.key)?;
            let mut held = HeldKeys::default();
            // One press per *distinct* modifier. The wire format is a list,
            // so `shift+shift+...+a` is a legal (if pointless) request from
            // a client that builds one by concatenation; without this, a
            // long enough one would walk the keymap once per repeat and
            // overrun `HeldKeys`' fixed buffer. Matched exhaustively rather
            // than cast from the enum's discriminant, so adding a modifier
            // is a compile error here instead of a bit that silently
            // aliases another one's.
            let mut seen = 0u8;
            // The probe walks the whole keymap (a few hundred FFI calls,
            // no heap -- see `ModifierKeys::probe`), so it is built lazily
            // and shared by every modifier in the combo, the same way
            // `type_text` shares one across a whole string. `msg key` is
            // per-request IPC, not a per-frame path, so this adds no new
            // cost class over what `type` already pays.
            let mut modifier_keys = None;
            for &modifier in &combo.modifiers {
                let bit = match modifier {
                    Modifier::Ctrl => 1 << 0,
                    Modifier::Shift => 1 << 1,
                    Modifier::Alt => 1 << 2,
                    Modifier::Super => 1 << 3,
                };
                if seen & bit != 0 {
                    continue;
                }
                seen |= bit;
                // Asked of the keymap, not of a hard-coded `_L` keysym: a
                // layout that moved the real modifier elsewhere (or never
                // had it on the left-hand key) still resolves, and one with
                // no holdable key for it is refused with the same honest
                // error as before.
                let code = modifiers::modifier_key(keymap, layout, &mut modifier_keys, modifier)
                    .ok_or_else(|| format!("no key for `{}` in this layout", modifier.name()))?;
                held.push(code);
            }
            Ok((code, held))
        })
    }

    /// Types text by pressing whichever keys produce those characters, with
    /// whatever the active layout needs held down around each one -- Shift
    /// for `A` and `!`, AltGr for a German layout's `@`, nothing at all for
    /// the level-0 characters that make up most text.
    ///
    /// A character no single keypress produces -- `é` on a `de` layout, `~`
    /// on a Nordic one -- goes through a second path instead of failing: the
    /// dead-key sequence from the session-locale compose table (see the
    /// `compose` module), pressed as the two keypresses a person would
    /// type. What stays refused, loudly: a character with no sequence on the
    /// active layout (plain `us` has no dead keys, so `é` is still "no
    /// key"), one that lives on an inactive layout group, and one only a
    /// locking or latching modifier reaches.
    ///
    /// Errors on the first character the layout cannot produce, leaving
    /// everything before it typed. That is deliberate, and the same trade
    /// this function has always made: the alternative is resolving the whole
    /// string up front so it is all-or-nothing, which buys atomicity for a
    /// case an agent hits by mistyping a request, at the cost of a per-call
    /// buffer on the IPC path. Loud and partial beats silent and wrong,
    /// which is what this used to be: every shifted character came out as
    /// its unshifted twin, with no error at all.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        // Answered once, up front, rather than folded into the per-character
        // keymap lookup: "there is no keyboard" is not something the layout
        // could ever say, and reporting it as `Untypable::NoKey` would make
        // it indistinguishable from "this layout has no `é`".
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| NO_KEYBOARD.to_owned())?;
        // Filled in by the first character that needs a modifier held (see
        // `modifiers::plan`), so a lowercase string never pays for the
        // keymap walk and a mixed one pays for it once, not per character.
        let mut modifier_keys = None;
        // The compose fallback's per-request table (see `compose`), built on
        // the first character the direct path cannot type and shared by the
        // rest of the string. `None` until then, so directly typable text --
        // the overwhelmingly common case -- never pays for the table build
        // or the scan; `Some(None)` remembers that this session has no
        // usable table, so a string of many untypable characters fails fast
        // after the first lookup instead of re-reading the locale per
        // character.
        let mut composed: Option<Option<compose::SequenceMap>> = None;
        for character in text.chars() {
            let keysym = keysym_for_char(character);
            let direct = self.with_keymap(&keyboard, |keymap, layout| {
                modifiers::plan(keymap, layout, keysym, &mut modifier_keys)
            });
            // Exactly one keypress for the common case; the dead key plus
            // its base for a composed character. Both halves of a sequence
            // are resolved before either is pressed (see `plan_sequence`),
            // so one character is all-or-nothing even though the string as
            // a whole is prefix-typed.
            let (first, second) = match direct {
                Ok(plan) => (plan, None),
                Err(reason) => {
                    match self.compose_plan(&keyboard, character, &mut composed, &mut modifier_keys)
                    {
                        Some((dead, base)) => (dead, Some(base)),
                        None => {
                            return Err(match reason {
                                Untypable::NoKey => {
                                    format!("no key for `{character}` in this layout")
                                }
                                Untypable::NoModifiers => format!(
                                    "`{character}` needs a modifier this layout only locks or latches"
                                ),
                            });
                        }
                    }
                }
            };
            // A keybinding firing mid-string here would be surprising --
            // `type_text` is meant to simulate typed characters, not chords
            // -- but every character is sent with exactly the modifiers its
            // own level needs and no others, so this only happens when a
            // binding really is on that combination. The modifier presses
            // are checked too, not just the character's own: Smithay updates
            // the xkb state before running this filter, so a modifier's own
            // press already carries itself in `mods` by the time the binding
            // table sees it -- which means a bind written `shift+shift_l`
            // matches it (a *bare* `shift_l` bind, by the same token, never
            // can). Such a bind swallows the press the *client* needs to see
            // to decode this character as a capital, which is the same
            // silently-wrong-text failure this whole path exists to stop.
            // Warn rather than let any of it happen with no signal.
            let mut intercepted = self.press_plan(&first);
            if let Some(second) = &second {
                intercepted |= self.press_plan(second);
            }
            if intercepted {
                tracing::warn!(%character, "a keybinding intercepted a character from type_text");
            }
        }
        Ok(())
    }

    /// Presses and releases one planned key with its modifiers held around
    /// it, answering whether a keybinding intercepted any of it. Split out
    /// of [`State::type_text`] so a composed character's two halves share
    /// the press/release shape with a direct character's one.
    fn press_plan(&mut self, plan: &KeyPlan) -> bool {
        let mut intercepted = false;
        for &code in plan.modifiers.as_slice() {
            intercepted |= self.key(code, KeyState::Pressed).intercepted;
        }
        intercepted |= self.key(plan.code, KeyState::Pressed).intercepted;
        self.key(plan.code, KeyState::Released);
        for &code in plan.modifiers.as_slice().iter().rev() {
            self.key(code, KeyState::Released);
        }
        intercepted
    }

    /// The dead-key sequence that types `character`, or `None` when the
    /// active layout has none (in which case the caller reports its direct
    /// refusal, still the honest answer).
    ///
    /// `composed` is the per-request cache described at its declaration:
    /// built once, on the first character that needs it, from the session
    /// locale's compose table over the active layout. `modifier_keys` is the
    /// shared probe cache both resolution paths plan through.
    fn compose_plan(
        &mut self,
        keyboard: &KeyboardHandle<Self>,
        character: char,
        composed: &mut Option<Option<compose::SequenceMap>>,
        modifier_keys: &mut Option<modifiers::ModifierKeys>,
    ) -> Option<(KeyPlan, KeyPlan)> {
        if composed.is_none() {
            *composed = Some(self.with_keymap(keyboard, |keymap, layout| {
                compose::table_from_session_locale()
                    .map(|table| compose::build_map(keymap, layout, &table))
            }));
        }
        let map = composed.as_ref()?.as_ref()?;
        self.with_keymap(keyboard, |keymap, layout| {
            compose::plan_sequence(keymap, layout, map, character, modifier_keys)
        })
    }

    /// Releases every key this seat believes held: what a keyboard that
    /// lost focus without delivering its releases needs (`--nested`'s host
    /// keyboard `leave` -- an Alt+Tab in the host keeps Alt's release on the
    /// host side). Without it the seat's modifier state stays held, which
    /// with `[floating] modifier` set to that key turns every click on a
    /// floating window into a drag. Each goes through [`State::key`], so a
    /// release a binding intercepted is still intercepted and the focused
    /// client sees the rest. Rare (a focus change), so the snapshot's
    /// allocation is fine.
    pub(super) fn release_held_keys(&mut self) {
        let held: Vec<Keycode> = self.held_keys.iter().copied().collect();
        for keycode in held {
            self.key(keycode, KeyState::Released);
        }
    }

    /// Presses the modifiers among `evdev` (a `wl_keyboard.enter`'s key
    /// array) that this seat does not believe held, so a modifier held as
    /// focus arrives counts. Modifiers only: pressing an ordinary held key
    /// would deliver a keystroke nobody typed. And without binding
    /// dispatch: a bind on a bare modifier (`Super_L`) must not fire.
    /// Matched by evdev code against the default layout the seat uses (see
    /// `nested_dispatch.rs`).
    pub(super) fn press_held_modifiers(&mut self, evdev: &[u32]) {
        // Ctrl, Shift, Alt and Meta, left and right.
        const MODIFIERS: [u32; 8] = [29, 97, 42, 54, 56, 100, 125, 126];
        for &code in evdev.iter().filter(|code| MODIFIERS.contains(code)) {
            let keycode = Keycode::new(code + 8);
            if !self.held_keys.contains(&keycode) {
                // No bindings: this re-states a key already down, it is not
                // a keystroke (see `key_with`).
                self.key_with(keycode, KeyState::Pressed, false);
            }
        }
    }

    /// `pub(super)` rather than private: `nested_dispatch.rs` forwards real
    /// host keyboard events through this exact same path IPC-injected key
    /// presses already use, rather than duplicating the `keyboard.input`
    /// call.
    pub(super) fn key(&mut self, keycode: Keycode, state: KeyState) -> KeyOutcome {
        self.key_with(keycode, state, true)
    }

    /// [`State::key`], with keybinding dispatch on (`bindings`) or off. Off
    /// is for a press that re-states a key already down rather than a
    /// keystroke (`press_held_modifiers`): a `Super_L = "spawn ..."`
    /// launcher bind must not fire because focus came back with Super held.
    fn key_with(&mut self, keycode: Keycode, state: KeyState, bindings: bool) -> KeyOutcome {
        // Announced before the keyboard check, not after: a key event with
        // no keyboard on the seat reaches no client, but it is still a
        // user at the machine rather than an idle one.
        self.announce_activity();
        let Some(keyboard) = self.seat.get_keyboard() else {
            return KeyOutcome::default();
        };
        let serial = SERIAL_COUNTER.next_serial();
        // Who this key is *about* to go to, read before the filter below runs
        // rather than after: a keybinding can change focus, and this key's
        // serial must not follow it onto whatever gained it. What the filter
        // decides then says whether it went there at all -- see the recording
        // after the call.
        let recipient = keyboard.current_focus().and_then(|focus| {
            focus
                .wl_surface()
                .and_then(|surface| self.client_of(&surface))
        });
        let transition = self.note_held(keycode, state);
        let time = InputTime::from_millis(self.millis());
        let outcome = keyboard
            .input::<KeyOutcome, _>(self, keycode, state, serial, time, |data, mods, handle| {
                match state {
                    KeyState::Pressed if !bindings => FilterResult::Forward,
                    KeyState::Pressed => {
                        // The unshifted (level 0) symbol: see the module
                        // comment on `keybindings` for why this, not
                        // `modified_sym()`.
                        let Some(&keysym) = handle.raw_syms().first() else {
                            return FilterResult::Forward;
                        };
                        let Some(bound) = data.keybindings.match_key(keysym, mods.into()) else {
                            return FilterResult::Forward;
                        };
                        // While the session is locked, a keybinding that runs
                        // an `Action` must not fire: `spawn` would put a
                        // terminal on top of the lock screen, `close`/`quit`
                        // would reach through it, and every layout action
                        // would move windows the user cannot see. Forwarded
                        // rather than swallowed, so the combination is just a
                        // keystroke the lock client receives like any other
                        // -- and deliberately *before* `suppressed_keys`, so
                        // the matching release is forwarded too rather than
                        // eaten as a stale entry.
                        //
                        // `ChangeVt` is the one exception, on purpose: it is
                        // a session-level escape hatch, not a way into this
                        // session (the VT it switches to has its own login),
                        // and it is the recovery path when a lock client
                        // wedges. See `session_lock.rs`.
                        if data.session_lock.is_locked() && !matches!(bound, Bound::ChangeVt(_)) {
                            return FilterResult::Forward;
                        }
                        // Remember this keycode was intercepted so the
                        // matching release is intercepted too, rather than
                        // forwarded to whatever gains focus in between (e.g.
                        // after this action closes the current focus) as a
                        // spurious lone release it never pressed.
                        data.suppressed_keys.insert(keycode);
                        let vt_switch = match bound {
                            Bound::Action(action) => {
                                data.act(action);
                                None
                            }
                            Bound::ChangeVt(vt) => Some(data.change_vt(vt)),
                        };
                        FilterResult::Intercept(KeyOutcome {
                            intercepted: true,
                            vt_switch,
                        })
                    }
                    KeyState::Released => {
                        if data.suppressed_keys.remove(&keycode) {
                            FilterResult::Intercept(KeyOutcome {
                                intercepted: true,
                                vt_switch: None,
                            })
                        } else {
                            FilterResult::Forward
                        }
                    }
                }
            })
            .unwrap_or_default();
        // Only what was actually *delivered*, which is why this is after the
        // call and not before it: a key a binding intercepted never reaches
        // the focused client, and recording it would put a serial that client
        // never saw under its name -- guessable from the ones it did see (the
        // modifier presses around a chord are forwarded), which is exactly the
        // hole the client half of this check exists to close. The intercepted
        // *release* is worse still: by then `act` may have moved focus, so it
        // would land under a client that not only never received it but was
        // not even the intended recipient. A key Smithay absorbed as a
        // non-transition reached nobody either.
        //
        // Both states otherwise, because which of them a client mints an
        // activation token from is its own choice (a GTK button activates on
        // release). Nothing at all is recorded when nothing has focus: an
        // event no client received is evidence for no one. See
        // `interaction.rs`.
        if transition
            && !outcome.intercepted
            && let Some(client) = recipient
        {
            self.interaction_serials.record(serial, client);
        }
        outcome
    }

    /// Tracks this keycode's held state, answering whether the event actually
    /// changes it -- which is what decides whether Smithay delivers it at all.
    ///
    /// A mirror of the pinned rev's `KbdInternal::key_input`, which absorbs a
    /// press of a key already held and a release of a key it has no record of,
    /// returning `None` from `KeyboardHandle::input` *before* the filter and
    /// before forwarding. That `None` is indistinguishable from "the filter
    /// said forward", so [`KeyOutcome`] alone cannot tell a delivered key from
    /// an absorbed one, and neither can `pressed_keys()` (it clones a
    /// `HashSet` per call, which this path must not do). Both absorbed cases
    /// are reachable here: a lone release arrives under `--nested` for an
    /// ordinary key held as the keyboard entered scoot's window
    /// (`nested_dispatch` re-states only the modifiers of `enter`'s held-key
    /// array, see `press_held_modifiers`), and under `--tty` when a press
    /// lands while the session is paused.
    ///
    /// Invariant, same as [`State::suppressed_keys`]: this stays in step with
    /// Smithay's own set only because [`State::key`] (through `key_with`) is
    /// the one caller of
    /// `KeyboardHandle::input` in this compositor. Anything that ever feeds
    /// the seat keyboard another way (`input_forward`, `release_source`) must
    /// update this too, or a real key will be mistaken for an absorbed one.
    ///
    /// [`State::suppressed_keys`]: super::State::suppressed_keys
    fn note_held(&mut self, keycode: Keycode, state: KeyState) -> bool {
        match state {
            KeyState::Pressed => self.held_keys.insert(keycode),
            KeyState::Released => self.held_keys.remove(&keycode),
        }
    }

    /// Runs `read` against the seat keyboard's keymap and the layout (xkb
    /// group) currently in effect.
    ///
    /// Everything read through here is static keymap data, not live keyboard
    /// state: the answer is "what does this layout say", not "what is held
    /// right now". The active *layout* is the one exception, and it is read
    /// here rather than passed in precisely so every question about the
    /// keymap is asked of the same group -- the group the client will decode
    /// the resulting keypress in.
    ///
    /// The single place the raw keymap is borrowed, so the reasoning for
    /// that lives here once rather than at each call site.
    fn with_keymap<T>(
        &mut self,
        keyboard: &KeyboardHandle<Self>,
        // `FnMut`, not `FnOnce`, only because Smithay's `with_xkb_state`
        // asks for one; every caller here is run exactly once.
        mut read: impl FnMut(&xkb::Keymap, xkb::LayoutIndex) -> T,
    ) -> T {
        keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().expect("xkb state");
            let layout = xkb.active_layout().0;
            // SAFETY: the pinned Smithay rev's contract on `Xkb::keymap` is
            // that no ref-count on the keymap may outlive the `Xkb`. This
            // borrow is confined to the call below: `read` returns an owned
            // value that borrows nothing from the keymap, and the scratch
            // `xkb::State` `modifiers::plan` may build from it is dropped
            // before this closure returns. The keymap is only read, and the
            // keyboard's own state is left untouched (nothing here reports
            // `mods_changed`, so Smithay has nothing to broadcast on the way
            // out).
            let keymap = unsafe { xkb.keymap() };
            read(keymap, layout)
        })
    }

    /// Clicking focuses what is under the pointer, which the core then tracks.
    ///
    /// Walks the same front-to-back order `surface_under` does, because
    /// "what did the user click" has exactly one answer and it is whatever is
    /// drawn on top. Two rules come out of that:
    ///
    /// - A click on a layer-shell surface never moves *window* focus. The
    ///   window under a bar is not what the user clicked, and activating it
    ///   would move the focus ring for a click that never reached a window.
    /// - A click is also how keyboard focus leaves an `on_demand` layer
    ///   surface again -- on a window, on a bar that doesn't want keys, or on
    ///   bare desktop. The one thing it cannot do is take the keyboard off an
    ///   `exclusive` surface, which by protocol keeps it until it unmaps;
    ///   `layer_keyboard_focus` answers that before `clicked_layer` is ever
    ///   consulted, so nothing here has to special-case it.
    fn focus_under_pointer(&mut self) {
        // Nothing a click can focus exists while the session is locked:
        // window focus must not move behind the lock screen, and the lock
        // surface already holds the keyboard. Clicking it is how a password
        // field gets typed into, and that needs no focus change at all.
        if self.session_lock.is_locked() {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let location = pointer.current_location();
        if let Some(layer) = self.layer_under(&layer_shell::ABOVE_WINDOWS, location) {
            self.click_layer(&layer);
            return;
        }
        // A click in an override-redirect X window -- a menu item, a
        // drop-down entry -- focuses nothing: the window is not a window
        // scoot manages, and the click must not reach the one beneath it.
        #[cfg(feature = "xwayland")]
        if self.x11_unmanaged_under(location).is_some() {
            return;
        }
        // The same output-confined search the pointer focus uses (see
        // `output_clip.rs`): a click lands on what is drawn under it, never
        // on another output's window overhanging this one.
        if let Some(window) = self.window_element_under(location).map(|(w, _)| w.clone()) {
            let found = self
                .windows
                .iter()
                .find(|(_, candidate)| **candidate == window)
                .map(|(&id, _)| id);
            if let Some(id) = found {
                self.clicked_layer = None;
                // Ends in `apply()`, and so in `refresh_keyboard_focus()`,
                // which is what hands the keyboard back to this window even
                // when it was already the focused one.
                self.act(Action::FocusWindowId(id));
                return;
            }
            // A `Space` element this compositor doesn't know as a window is
            // not something to focus, but it is still a click on a window
            // rather than on the desktop: leave everything as it was, the
            // same as before layer-shell focus existed.
            return;
        }
        if let Some(layer) = self.layer_under(&layer_shell::BELOW_WINDOWS, location) {
            self.click_layer(&layer);
            return;
        }
        // Bare desktop: nothing to focus, but clicking it is a legitimate way
        // to dismiss an `on_demand` layer surface.
        self.clicked_layer = None;
        self.refresh_keyboard_focus();
    }
}

/// Clamps a pointer target to the union of every output's logical geometry
/// (milestone 19, phase D).
///
/// The union, not the output under the pointer: the milestone's focus
/// decision is that the pointer position picks the output, and a clamp to
/// the output the pointer is already on would make that unreachable --
/// relative motion past the first output's edge would clamp back onto it,
/// trapping the pointer on one screen forever. The union lets motion cross
/// onto the next output, and the existing focus derivation then applies
/// there unchanged.
///
/// The union is a bounding box, not a union of pixels: with uneven outputs
/// it contains dead zones over no output, where the pointer may rest and
/// where [`State::output_under`] misses exactly as an off-output absolute
/// move does. Bounds are half-open like [`State::output_under`]'s, so the
/// seam pixel between two adjacent outputs belongs to the one on its
/// right/below and the far edge clamps to `right - 1`.
///
/// Logical, not physical: the pointer lives in logical coordinates
/// (`pointer_move`, the core and every surface all use them), while
/// `current_mode()` is the physical framebuffer size. Clamping against the
/// physical size at a scale != 1 lets the pointer be driven past the last
/// logical pixel. [`State::space`]'s geometries are the same logical
/// rectangles the core lays out in (see [`logical_size`], which agrees with
/// them by construction), so the clamp can never disagree with what is on
/// screen.
///
/// Costs one geometry lookup per output -- on the per-motion hot path, but
/// a linear scan over a handful of outputs against the hit test,
/// constraint resolution and socket writes every motion already pays (see
/// `move_absolute`'s measured ranges). A single output takes the old
/// per-extent clamp exactly (see below), so one-output sessions neither
/// behave nor measure differently.
///
/// [`State::space`]: super::State::space
impl State {
    fn clamp_to_output_union(&self, x: f64, y: f64) -> (f64, f64) {
        if self.outputs.len() == 1 {
            // The fast path, and the one-screen `--tty` shape: relative motion
            // is `--tty` libinput's. The old expression exactly -- the
            // primary's logical extent through `clamp_to_extent` -- so the
            // single-output motion path keeps its measured cost (release, dev
            // VM, 200k `pointer_move_relative` x5: 561ns/event median before,
            // 665ns without this branch, back to overlapping after) as well as
            // its behavior. The branch predicts perfectly in production: the
            // output count only changes on a hotplug.
            let (width, height) = self.outputs.primary().map(logical_size).unwrap_or((0, 0));
            return (clamp_to_extent(x, width), clamp_to_extent(y, height));
        }
        let Some(union) = self.output_union() else {
            // No output yet: the old path read a `(0, 0)` extent here, which
            // clamps every coordinate to `0.0`.
            return (0.0, 0.0);
        };
        let (left, top) = (union.loc.x, union.loc.y);
        (
            clamp_to_extent(x - f64::from(left), union.size.w) + f64::from(left),
            clamp_to_extent(y - f64::from(top), union.size.h) + f64::from(top),
        )
    }

    /// The bounding box of every output's logical geometry -- the desktop's
    /// extent, which relative motion is clamped into and absolute devices
    /// (tablets, vfkit's digitizer) are mapped across. `None` before any
    /// output exists. Uneven outputs leave dead zones inside the box, which
    /// is the documented clamp shape (see [`State::clamp_to_output_union`]).
    ///
    /// Saturating: the sum is config-derived, not client-derived, and a
    /// saturated edge merely stacks two outputs rather than wrapping one into
    /// negative coordinates (same reasoning as `add_output`'s). No
    /// allocation: one pass over at most `MAX_OUTPUTS` outputs.
    pub(super) fn output_union(&self) -> Option<Rectangle<i32, Logical>> {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for output in self.outputs.iter() {
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            let right = geometry.loc.x.saturating_add(geometry.size.w);
            let bottom = geometry.loc.y.saturating_add(geometry.size.h);
            bounds = Some(match bounds {
                Some((left, top, right_edge, bottom_edge)) => (
                    left.min(geometry.loc.x),
                    top.min(geometry.loc.y),
                    right_edge.max(right),
                    bottom_edge.max(bottom),
                ),
                None => (geometry.loc.x, geometry.loc.y, right, bottom),
            });
        }
        bounds.map(|(left, top, right, bottom)| {
            Rectangle::new(
                (left, top).into(),
                (right.saturating_sub(left), bottom.saturating_sub(top)).into(),
            )
        })
    }
}

/// Clamps a coordinate to `[0, extent)`, `extent` being a dimension in
/// logical pixels (so `extent == 0` -- no output yet -- clamps everything to
/// `0`, same as the pointer starting at the origin before any output exists).
/// Logical, not physical: see [`State::clamp_to_output_union`] on why the two
/// differ once an output scale is set.
/// Pulled out of `pointer_move_relative` so it's testable without a live
/// seat, same rationale as `first_free` in `nested/buffers.rs`.
///
/// [`State::clamp_to_output_union`]: super::State::clamp_to_output_union
fn clamp_to_extent(value: f64, extent: i32) -> f64 {
    value.clamp(0.0, (extent - 1).max(0) as f64)
}

/// The Linux `BTN_*` code Wayland carries for a button (`pub(super)` for
/// the floating grab, which ends on its own button's release).
pub(super) fn button_code(button: PointerButton) -> u32 {
    match button {
        PointerButton::Left => BTN_LEFT,
        PointerButton::Right => BTN_RIGHT,
        PointerButton::Middle => BTN_MIDDLE,
    }
}

/// The key [`State::press`] may press for a name, or the message explaining
/// why this layout has none it may press.
///
/// The refusal is the point. `press` holds only the modifiers its caller
/// named, so a keysym that lives above the unmodified level has no honest
/// answer here: the key carrying it types something else when pressed bare,
/// and guessing which modifiers to add would be `type_text`'s job done
/// badly. `at` on a German layout isn't even expressible as a combo -- its
/// modifier is AltGr, which `Modifier` has no name for -- which is why the
/// message points at `type` rather than promising a spelling that works.
fn unmodified_key(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    keysym: Keysym,
    name: &str,
) -> Result<Keycode, String> {
    match modifiers::named_key(keymap, layout, keysym) {
        NamedKey::Unmodified(code) => Ok(code),
        NamedKey::OnlyModified => Err(format!(
            "`{name}` is not on this layout's unmodified level, and `key` holds only the \
             modifiers you name, so pressing it would type a different character; name the \
             unmodified key and its modifiers instead (`shift+1`, not `exclam`), or use `type`, \
             which works the modifiers out from the layout"
        )),
        NamedKey::Absent => Err(format!("no key for `{name}` in this layout")),
    }
}

/// Maps a character to the keysym that types it. `xkb::utf32_to_keysym` only
/// covers printable Latin-1 and a table of extra graphic Unicode mappings, so
/// C0 controls like `\n` fall through it entirely -- not to `Return`, to
/// nothing, which used to make `type_text("ls\n")` error out on the newline
/// and silently drop the rest of the string. Handle the controls a caller
/// would plausibly send before falling back to the general mapping.
fn keysym_for_char(character: char) -> Keysym {
    match character {
        '\n' | '\r' => Keysym::Return,
        '\t' => Keysym::Tab,
        _ => xkb::utf32_to_keysym(character as u32),
    }
}

/// `pub(super)`: also used by `config.rs` to resolve a `[binds]` combo's key
/// name.
pub(super) fn keysym_named(name: &str) -> Option<Keysym> {
    let exact = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    let keysym = if exact.raw() == 0 {
        xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE)
    } else {
        exact
    };
    (keysym.raw() != 0).then_some(keysym)
}
