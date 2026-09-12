//! Injected input: what an agent sends in place of a keyboard and mouse.

use flexwm_core::Action;
use flexwm_ipc::{KeyCombo, Modifier, PointerButton};
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode, Keysym, xkb};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use super::State;
use super::keybindings::Bound;
use super::tty::VtSwitchOutcome;

// Linux input event codes, which is what Wayland carries.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// What handling one key press/release actually did. `Default` gives the
/// right answer (`intercepted: false, vt_switch: None`) for the
/// no-keyboard-yet early return in [`State::key`] and for `keyboard.input`'s
/// own `None` case (no active keyboard focus target) -- both are "nothing
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
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let location = Point::<f64, Logical>::from((x, y));
        let under = self.surface_under(location);
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
        let (width, height) = self
            .output
            .as_ref()
            .and_then(|output| output.current_mode())
            .map(|mode| (mode.size.w, mode.size.h))
            .unwrap_or((0, 0));
        let current = pointer.current_location();
        let x = clamp_to_extent(current.x + dx, width);
        let y = clamp_to_extent(current.y + dy, height);
        self.pointer_move(x, y);
    }

    pub fn pointer_button(&mut self, button: PointerButton, pressed: bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if pressed {
            self.focus_under_pointer();
        }
        let serial = SERIAL_COUNTER.next_serial();
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
    }

    pub fn scroll(&mut self, dx: f64, dy: f64) {
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
    /// If `combo` matches a keybinding, `key` runs it exactly as a real
    /// keypress would -- this doesn't bypass keybindings the way
    /// `Request::Action` does. That's deliberate: an agent using `press` is
    /// asking for input-level fidelity, and `Request::Action` already exists
    /// as the direct, binding-independent way to invoke a window-management
    /// action.
    ///
    /// Returns whatever `key()` reports for the *main* key's press --
    /// modifiers alone can never match a keybinding (every binding in this
    /// table requires a non-modifier main key), so only that one call can
    /// ever carry a [`VtSwitchOutcome`]. `ipc.rs`'s `Request::Key` handler
    /// uses this to warn a caller whose only input/output is this IPC
    /// connection when a switch-away just made the compositor unreachable.
    pub fn press(&mut self, combo: &KeyCombo) -> Result<Option<VtSwitchOutcome>, String> {
        let keysym =
            keysym_named(&combo.key).ok_or_else(|| format!("unknown key `{}`", combo.key))?;
        // Resolve every keycode -- the main key and all modifiers -- before
        // pressing anything. `keycode_for` is a pure lookup against the
        // static keymap (no dependency on what's currently held), so doing
        // this up front means a name that doesn't exist on this layout is
        // rejected before any key state changes, rather than after some
        // modifiers are already down. Otherwise a failure partway through
        // would leave a modifier (e.g. Super) permanently held at the seat
        // level, silently corrupting every keypress after it.
        let code = self
            .keycode_for(keysym)
            .ok_or_else(|| format!("no key for `{}` in this layout", combo.key))?;
        let mut modifier_codes = Vec::with_capacity(combo.modifiers.len());
        for modifier in &combo.modifiers {
            let code = self
                .keycode_for(modifier_keysym(*modifier))
                .ok_or_else(|| format!("no key for `{modifier:?}` in this layout"))?;
            modifier_codes.push(code);
        }
        let mut held = Vec::with_capacity(modifier_codes.len());
        for code in modifier_codes {
            self.key(code, KeyState::Pressed);
            held.push(code);
        }
        let vt_switch = self.key(code, KeyState::Pressed).vt_switch;
        self.key(code, KeyState::Released);
        for code in held.into_iter().rev() {
            self.key(code, KeyState::Released);
        }
        Ok(vt_switch)
    }

    /// Types text by pressing whichever keys produce those characters.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        for character in text.chars() {
            let keysym = keysym_for_char(character);
            let Some(code) = self.keycode_for(keysym) else {
                return Err(format!("no key for `{character}` in this layout"));
            };
            let shift = if self.needs_shift(code, keysym) {
                self.keycode_for(Keysym::Shift_L)
            } else {
                None
            };
            if let Some(shift) = shift {
                self.key(shift, KeyState::Pressed);
            }
            // A keybinding firing mid-string here would be surprising --
            // `type_text` is meant to simulate typed characters, not chords
            // -- but it's only ever possible if a future binding needs no
            // modifiers at all, since every char here is sent with exactly
            // the modifiers (at most Shift) needed to produce it. Warn
            // rather than silently let it happen with no signal.
            if self.key(code, KeyState::Pressed).intercepted {
                tracing::warn!(%character, "a keybinding intercepted a character from type_text");
            }
            self.key(code, KeyState::Released);
            if let Some(shift) = shift {
                self.key(shift, KeyState::Released);
            }
        }
        Ok(())
    }

    /// `pub(super)` rather than private: `nested_dispatch.rs` forwards real
    /// host keyboard events through this exact same path IPC-injected key
    /// presses already use, rather than duplicating the `keyboard.input`
    /// call.
    pub(super) fn key(&mut self, keycode: Keycode, state: KeyState) -> KeyOutcome {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return KeyOutcome::default();
        };
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        keyboard
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
            .unwrap_or_default()
    }

    fn keycode_for(&self, keysym: Keysym) -> Option<Keycode> {
        self.seat.get_keyboard()?.keycode_for_keysym(keysym)
    }

    /// Whether a keysym sits above the first level of its key, i.e. needs Shift.
    fn needs_shift(&mut self, keycode: Keycode, keysym: Keysym) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().expect("xkb state");
            let layout = xkb.active_layout();
            let symbols = xkb.raw_syms_for_key_in_layout(keycode, layout);
            symbols.first() != Some(&keysym) && symbols.contains(&keysym)
        })
    }

    /// Clicking focuses what is under the pointer, which the core then tracks.
    fn focus_under_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some((window, _)) = self.space.element_under(pointer.current_location()) else {
            return;
        };
        let window = window.clone();
        let Some((&id, _)) = self
            .windows
            .iter()
            .find(|(_, candidate)| **candidate == window)
        else {
            return;
        };
        self.act(Action::FocusWindowId(id));
    }
}

/// Clamps a coordinate to `[0, extent)`, `extent` being a dimension in
/// pixels (so `extent == 0` -- no output yet -- clamps everything to `0`,
/// same as the pointer starting at the origin before any output exists).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_and_friends_map_to_named_keys_not_utf32() {
        assert_eq!(keysym_for_char('\n'), Keysym::Return);
        assert_eq!(keysym_for_char('\r'), Keysym::Return);
        assert_eq!(keysym_for_char('\t'), Keysym::Tab);
    }

    #[test]
    fn printable_ascii_falls_back_to_utf32_to_keysym() {
        // Untouched by the control-character special case, so this should
        // still go through the general xkbcommon mapping.
        assert_eq!(keysym_for_char('a'), xkb::utf32_to_keysym('a' as u32));
        assert_eq!(keysym_for_char('!'), xkb::utf32_to_keysym('!' as u32));
    }

    #[test]
    fn keysym_named_accepts_exact_and_case_insensitive_names() {
        assert_eq!(keysym_named("Return"), Some(Keysym::Return));
        assert_eq!(keysym_named("return"), Some(Keysym::Return));
        assert_eq!(keysym_named("ctrl+shift+t"), None);
        assert_eq!(keysym_named("not-a-real-key"), None);
    }

    #[test]
    fn button_codes_match_linux_input_event_codes() {
        assert_eq!(code(PointerButton::Left), BTN_LEFT);
        assert_eq!(code(PointerButton::Right), BTN_RIGHT);
        assert_eq!(code(PointerButton::Middle), BTN_MIDDLE);
    }

    #[test]
    fn clamp_to_extent_keeps_values_inside_the_output() {
        assert_eq!(clamp_to_extent(-5.0, 800), 0.0);
        assert_eq!(clamp_to_extent(5.0, 800), 5.0);
        assert_eq!(clamp_to_extent(900.0, 800), 799.0);
        // No output yet: everything clamps to the origin.
        assert_eq!(clamp_to_extent(50.0, 0), 0.0);
    }

    #[test]
    fn modifier_keysyms_are_the_left_variant() {
        assert_eq!(modifier_keysym(Modifier::Ctrl), Keysym::Control_L);
        assert_eq!(modifier_keysym(Modifier::Shift), Keysym::Shift_L);
        assert_eq!(modifier_keysym(Modifier::Alt), Keysym::Alt_L);
        assert_eq!(modifier_keysym(Modifier::Super), Keysym::Super_L);
    }
}
