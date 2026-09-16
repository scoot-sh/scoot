//! Injected input: what an agent sends in place of a keyboard and mouse.

use flexwm_core::Action;
use flexwm_ipc::{KeyCombo, Modifier, PointerButton};
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, KeyboardHandle, Keycode, Keysym, xkb};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use super::State;
use super::keybindings::Bound;
use super::layer_shell;
use super::output_scale::logical_size;
use super::tty::VtSwitchOutcome;
use modifiers::{HeldKeys, NamedKey, Untypable};

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
        let time = InputTime::from_millis(self.millis());
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
        // dirty here would cost a real render for no visible effect.
        // libinput can report motion at 500-1000Hz while the display flips
        // at ~60Hz; this only marks the frame dirty; `render()` still runs
        // at most once per frame tick, not once per event.
        if self.tty.is_some() {
            self.request_render();
        }
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

    /// Moves the pointer by a relative delta, clamped to the current
    /// output's bounds. `--tty`'s only source of pointer motion: unlike
    /// nested's host-forwarded motion (already absolute) or IPC's
    /// `pointer move X Y`, libinput reports relative dx/dy for a plain
    /// mouse, with no absolute position of its own -- this is what turns
    /// that into the same absolute `pointer_move` every other input source
    /// already uses. Kept backend-neutral like every other method here:
    /// nothing about it is `--tty`-specific, it just happens to be the one
    /// caller that needs it today.
    pub fn pointer_move_relative(&mut self, dx: f64, dy: f64) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // The *logical* output extent, not the physical one: the pointer
        // lives in logical coordinates (`pointer_move`, the core and every
        // surface all use them), while `current_mode()` is the physical
        // framebuffer size. Clamping against the physical size at a scale != 1
        // lets the pointer be driven past the last logical pixel -- off the
        // real desktop and, under `--tty`, onto a coordinate no output
        // contains. `logical_size` is the same rectangle the core and the
        // `Space` use, so the clamp can never disagree with what is on screen.
        let (width, height) = self.output.as_ref().map(logical_size).unwrap_or((0, 0));
        let current = pointer.current_location();
        let x = clamp_to_extent(current.x + dx, width);
        let y = clamp_to_extent(current.y + dy, height);
        self.pointer_move(x, y);
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
                button: code(button),
                state,
                serial,
                time,
            },
        );
        pointer.frame(self);
        // A click outside an open menu is what dismisses it (Smithay's
        // `PopupPointerGrab` does that inside the call above), and it leaves
        // the keyboard on the popup's root rather than wherever flexwm's own
        // policy would put it. Settling here rather than waiting for the
        // client's follow-up traffic keeps the two in step within the same
        // event. One `Option` check when no menu is open.
        self.settle_popup_grab();
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
    /// when pressed bare -- `flexwm msg key exclam` typed `1` for as long as
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
    /// never on a modifier's own keysym (`Control_L`/`Shift_L`/`Alt_L`/
    /// `Super_L`), so pressing one of this combo's modifiers on its own --
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
                let keysym = modifier_keysym(modifier);
                held.push(unmodified_key(keymap, layout, keysym, modifier.name())?);
            }
            Ok((code, held))
        })
    }

    /// Types text by pressing whichever keys produce those characters, with
    /// whatever the active layout needs held down around each one -- Shift
    /// for `A` and `!`, AltGr for a German layout's `@`, nothing at all for
    /// the level-0 characters that make up most text.
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
        for character in text.chars() {
            let keysym = keysym_for_char(character);
            let plan = self
                .with_keymap(&keyboard, |keymap, layout| {
                    modifiers::plan(keymap, layout, keysym, &mut modifier_keys)
                })
                .map_err(|reason| match reason {
                    Untypable::NoKey => format!("no key for `{character}` in this layout"),
                    Untypable::NoModifiers => {
                        format!("`{character}` needs a modifier this layout only locks or latches")
                    }
                })?;
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
            let mut intercepted = false;
            for &code in plan.modifiers.as_slice() {
                intercepted |= self.key(code, KeyState::Pressed).intercepted;
            }
            intercepted |= self.key(plan.code, KeyState::Pressed).intercepted;
            if intercepted {
                tracing::warn!(%character, "a keybinding intercepted a character from type_text");
            }
            self.key(plan.code, KeyState::Released);
            for &code in plan.modifiers.as_slice().iter().rev() {
                self.key(code, KeyState::Released);
            }
        }
        Ok(())
    }

    /// `pub(super)` rather than private: `nested_dispatch.rs` forwards real
    /// host keyboard events through this exact same path IPC-injected key
    /// presses already use, rather than duplicating the `keyboard.input`
    /// call.
    pub(super) fn key(&mut self, keycode: Keycode, state: KeyState) -> KeyOutcome {
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
        let recipient = keyboard
            .current_focus()
            .and_then(|surface| self.client_of(&surface));
        let transition = self.note_held(keycode, state);
        let time = InputTime::from_millis(self.millis());
        let outcome = keyboard
            .input::<KeyOutcome, _>(self, keycode, state, serial, time, |data, mods, handle| {
                match state {
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
    /// are reachable here: a lone release arrives under `--nested` when a
    /// modifier was held as the pointer entered flexwm's window
    /// (`nested_dispatch` forwards keys but not `enter`'s held-key array), and
    /// under `--tty` when a press lands while the session is paused.
    ///
    /// Invariant, same as [`State::suppressed_keys`]: this stays in step with
    /// Smithay's own set only because [`State::key`] is the one caller of
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
        if let Some(window) = self.space.element_under(location).map(|(w, _)| w.clone()) {
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

/// Clamps a coordinate to `[0, extent)`, `extent` being a dimension in
/// logical pixels (so `extent == 0` -- no output yet -- clamps everything to
/// `0`, same as the pointer starting at the origin before any output exists).
/// Logical, not physical: see `pointer_move_relative`'s comment on why the two
/// differ once an output scale is set.
/// Pulled out of `pointer_move_relative` so it's testable without a live
/// seat, same rationale as `first_free` in `nested/buffers.rs`.
fn clamp_to_extent(value: f64, extent: i32) -> f64 {
    value.clamp(0.0, (extent - 1).max(0) as f64)
}

fn code(button: PointerButton) -> u32 {
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

fn modifier_keysym(modifier: Modifier) -> Keysym {
    match modifier {
        Modifier::Ctrl => Keysym::Control_L,
        Modifier::Shift => Keysym::Shift_L,
        Modifier::Alt => Keysym::Alt_L,
        Modifier::Super => Keysym::Super_L,
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
