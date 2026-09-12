//! Injected input: what an agent sends in place of a keyboard and mouse.

use flexwm_core::Action;
use flexwm_ipc::{KeyCombo, Modifier, PointerButton};
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode, Keysym, xkb};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use super::State;

// Linux input event codes, which is what Wayland carries.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

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
    pub fn press(&mut self, combo: &KeyCombo) -> Result<(), String> {
        let keysym =
            keysym_named(&combo.key).ok_or_else(|| format!("unknown key `{}`", combo.key))?;
        let mut held = Vec::new();
        for modifier in &combo.modifiers {
            let code = self
                .keycode_for(modifier_keysym(*modifier))
                .ok_or_else(|| format!("no key for `{modifier:?}` in this layout"))?;
            self.key(code, KeyState::Pressed);
            held.push(code);
        }
        let code = self
            .keycode_for(keysym)
            .ok_or_else(|| format!("no key for `{}` in this layout", combo.key))?;
        self.key(code, KeyState::Pressed);
        self.key(code, KeyState::Released);
        for code in held.into_iter().rev() {
            self.key(code, KeyState::Released);
        }
        Ok(())
    }

    /// Types text by pressing whichever keys produce those characters.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        for character in text.chars() {
            let keysym = xkb::utf32_to_keysym(character as u32);
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
            self.key(code, KeyState::Pressed);
            self.key(code, KeyState::Released);
            if let Some(shift) = shift {
                self.key(shift, KeyState::Released);
            }
        }
        Ok(())
    }

    fn key(&mut self, keycode: Keycode, state: KeyState) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
            FilterResult::Forward
        });
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

fn keysym_named(name: &str) -> Option<Keysym> {
    let exact = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    let keysym = if exact.raw() == 0 {
        xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE)
    } else {
        exact
    };
    (keysym.raw() != 0).then_some(keysym)
}
