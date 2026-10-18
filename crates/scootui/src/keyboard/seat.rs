//! The seat: `wl_seat` versions, the keyboard event handlers, and one
//! timerfd's worth of repeat.
//!
//! Binding: [`negotiate_version`] takes the advertised version and
//! returns what to bind -- 9, or 10 where offered -- or `None` below 9.
//! Version 9 is the floor because everything here (keymap, key, modifiers
//! since 1, `repeat_info` since 4) predates it; 10 is the cap because the
//! only thing above it this module uses is the `repeated` key state, and
//! nothing above 10 touches the keyboard at all. Binding higher would buy
//! nothing and widen the protocol surface.
//!
//! Version 10 changes exactly one thing: the compositor *may* send
//! `repeated`-state keys, but only when it sent `repeat_info` with rate 0
//! (it takes over repetition then). So: at rate 0 the timer never arms
//! and `repeated` states become ticks; at rate nonzero the timer owns
//! repetition and stray `repeated` states are ignored (the compositor
//! should not send them then, but a buggy one must not double a key).
//!
//! The handlers take plain data, not Wayland objects, so every transition
//! below is a unit test without a compositor; [`dispatch_keyboard`] maps
//! the protocol events onto them for the app's `Dispatch` impl (three
//! lines: forward the event and the text buffer).

use std::os::unix::io::BorrowedFd;

use super::keymap::{KeymapError, KeymapSet, read_keymap_bytes};
use super::repeat::Repeater;
use super::state::Keyboard;

/// The lowest `wl_seat` version this module binds: everything it uses
/// (`keymap`, `key`, `modifiers`, `repeat_info`) is at or below it.
pub const MIN_SEAT_VERSION: u32 = 9;
/// The highest version bound: 10 adds the `repeated` key state, and
/// nothing above 10 touches the keyboard.
pub const MAX_SEAT_VERSION: u32 = 10;

/// The `xkb_v1` keymap format: the only `wl_keyboard.keymap` format that
/// carries a keymap (0 is `no_keymap`).
pub const KEYMAP_FORMAT_XKB_V1: u32 = 1;

/// What to bind for an advertised `wl_seat` version: `None` below
/// [`MIN_SEAT_VERSION`], the advertised version capped at
/// [`MAX_SEAT_VERSION`] otherwise.
pub fn negotiate_version(advertised: u32) -> Option<u32> {
    if advertised < MIN_SEAT_VERSION {
        None
    } else {
        Some(advertised.min(MAX_SEAT_VERSION))
    }
}

/// A `wl_keyboard.key` state, in protocol numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Released,
    Pressed,
    Repeated,
}

impl KeyState {
    pub(crate) fn from_wire(state: u32) -> Option<Self> {
        match state {
            0 => Some(Self::Released),
            1 => Some(Self::Pressed),
            2 => Some(Self::Repeated),
            _ => None,
        }
    }
}

/// One decoded key: its keysym and the bytes written into the caller's
/// buffer. `repeat` marks ticks (timer expirations at rate nonzero,
/// compositor `repeated` states at rate 0) as opposed to fresh presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyOutput {
    pub keysym: u32,
    pub text_len: usize,
    pub repeat: bool,
}

/// One seat's keyboard: decode state plus the repeat timer plus focus.
/// No Wayland objects: the app owns the `WlSeat`/`WlKeyboard` and forwards
/// events here (see [`dispatch_keyboard`]).
pub struct KeyboardSeat {
    decode: Option<Keyboard>,
    repeat: Repeater,
    focused: bool,
}

impl KeyboardSeat {
    /// No keymap yet (no output until the first one), no repeat, no focus.
    pub fn new() -> Self {
        Self {
            decode: None,
            repeat: Repeater::new(),
            focused: false,
        }
    }

    /// Decode dead keys through a Compose table built for `locale`
    /// (explicit, never from the environment: the launcher passes the
    /// session locale, tests pin `"C"`). A table that will not build
    /// degrades to direct decoding rather than failing the seat.
    pub fn set_compose_locale(&mut self, locale: &std::ffi::OsStr) {
        if let Some(decode) = self.decode.as_mut() {
            decode.set_compose_locale(locale);
        }
    }

    /// Test seam: decode without a compositor. The headless test uses the
    /// real `keymap_event` path instead; this exists for RMLVO-compiled
    /// unit layouts.
    pub fn set_keymap_set(&mut self, keymap: KeymapSet) {
        self.decode = Some(Keyboard::new(keymap));
        self.repeat.keymap_changed();
    }

    /// Whether a keymap has arrived. Until it has, no key decodes.
    pub fn has_keymap(&self) -> bool {
        self.decode.is_some()
    }

    /// A `wl_keyboard.keymap` event: compile the fd (seek, cap, refuse
    /// empties) and swap it in, keeping the old map when the new text
    /// fails. Any in-flight repeat ends either way: on success the
    /// keycodes may mean other keys now, and on failure the decode no
    /// longer matches the compositor's, so holding is never the right
    /// call. A failed swap otherwise changes nothing (the old map and
    /// its Compose state stand); a successful one rebuilds both.
    pub fn keymap_event(&mut self, format: u32, fd: BorrowedFd<'_>) -> Result<(), KeymapError> {
        if format != KEYMAP_FORMAT_XKB_V1 {
            return Err(KeymapError::UnsupportedFormat(format));
        }
        let text = read_keymap_bytes(fd);
        self.repeat.keymap_changed();
        let text = text?;
        match self.decode.as_mut() {
            Some(decode) => decode.replace_keymap(text)?,
            None => self.decode = Some(Keyboard::new(KeymapSet::compile(text)?)),
        }
        Ok(())
    }

    /// A `wl_keyboard.repeat_info` event: rate 0 disarms (the compositor
    /// may take over with `repeated` states); a change mid-hold re-arms
    /// from now through the repeater.
    pub fn repeat_info_event(&mut self, rate: i32, delay: i32) {
        self.repeat.set_repeat_info(rate, delay);
    }

    /// A `wl_keyboard.modifiers` event, straight into the XKB state. Kept
    /// across focus loss (the compositor sends a zeroed one when it wants
    /// a reset, per the protocol).
    pub fn modifiers_event(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(decode) = self.decode.as_mut() {
            decode.update_modifiers(depressed, latched, locked, group);
        }
    }

    /// A `wl_keyboard.key` event: decode, arm or cancel the repeat, and
    /// report the output. `text` is the caller's buffer (`text_len` bytes
    /// of it hold UTF-8 on `Some`); bring at least
    /// [`MIN_TEXT_BYTES`](super::state::MIN_TEXT_BYTES).
    ///
    /// Releases and presses of keys with no keysym report `Some` with an
    /// empty text (`text_len` 0): the app still needs the press itself
    /// (editing keys, shortcuts).
    pub fn key_event(
        &mut self,
        evdev: u32,
        state: KeyState,
        text: &mut [u8],
    ) -> Option<KeyOutput> {
        if !self.focused {
            return None;
        }
        let decode = self.decode.as_mut()?;
        match state {
            KeyState::Pressed => {
                let keysym = decode.keysym(evdev);
                let text_len = decode.text_into(evdev, text).unwrap_or(0);
                self.repeat.press(evdev, decode.key_repeats(evdev));
                Some(KeyOutput {
                    keysym,
                    text_len,
                    repeat: false,
                })
            }
            KeyState::Released => {
                let keysym = decode.keysym(evdev);
                self.repeat.release(evdev);
                // A release types nothing and feeds nothing (Compose
                // included): its keysym is still the app's, for the
                // shortcut layer that reads releases.
                Some(KeyOutput {
                    keysym,
                    text_len: 0,
                    repeat: false,
                })
            }
            KeyState::Repeated => {
                // Compositor-driven repetition (rate 0): the event's own
                // key, as itself, never touching Compose. At rate nonzero
                // the timer owns repetition and this state should not
                // arrive; ignoring it keeps a buggy compositor from
                // doubling a key.
                if self.repeat.rate() != 0 {
                    return None;
                }
                let keysym = decode.keysym(evdev);
                let text_len = decode.repeat_text_into(evdev, text).unwrap_or(0);
                Some(KeyOutput {
                    keysym,
                    text_len,
                    repeat: true,
                })
            }
        }
    }

    /// The repeat timer fired: emit one output per expiration for the
    /// held key. `None` when nothing is held (a disarm raced the poll).
    pub fn tick(&mut self, text: &mut [u8]) -> Option<(u32, KeyOutput)> {
        let expirations = self.repeat.take_expirations();
        if expirations == 0 {
            return None;
        }
        let held = self.repeat.held()?;
        if !self.focused {
            return None;
        }
        let decode = self.decode.as_mut()?;
        let keysym = decode.keysym(held);
        let text_len = decode.repeat_text_into(held, text).unwrap_or(0);
        Some((
            held,
            KeyOutput {
                keysym,
                text_len,
                repeat: true,
            },
        ))
    }

    /// Keyboard focus arrived (the `enter` event's surface is the app's
    /// own; routing surfaces is the app's job).
    pub fn enter(&mut self) {
        self.focused = true;
    }

    /// Keyboard focus left: the timer dies (a release held across the
    /// loss never arrives) and the Compose sequence resets.
    pub fn leave(&mut self) {
        self.focused = false;
        self.repeat.focus_lost();
        if let Some(decode) = self.decode.as_mut() {
            decode.reset_compose();
        }
    }

    /// Whether the seat's focus is here. Unfocused keys are dropped, not
    /// queued: a launcher must not type what was pressed elsewhere.
    pub fn has_focus(&self) -> bool {
        self.focused
    }

    /// Caps Lock's locked state (false without a keymap, or a layout
    /// without `Lock`).
    pub fn caps_lock(&self) -> bool {
        self.decode.as_ref().is_some_and(Keyboard::caps_lock)
    }

    /// Num Lock's locked state, same absence rules.
    pub fn num_lock(&self) -> bool {
        self.decode.as_ref().is_some_and(Keyboard::num_lock)
    }

    /// Whether the repeat timer is armed (the idle invariant: false with
    /// no repeatable key held).
    pub fn repeat_armed(&self) -> bool {
        self.repeat.armed()
    }

    /// The timerfd to poll, when armed.
    pub fn repeat_fd(&self) -> Option<BorrowedFd<'_>> {
        self.repeat.poll_fd()
    }
}

impl Default for KeyboardSeat {
    fn default() -> Self {
        Self::new()
    }
}
