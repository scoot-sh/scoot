//! Decoding: modifiers, keysyms and UTF-8 into a caller-supplied buffer.
//!
//! A [`Keyboard`] owns one compiled [`KeymapSet`] plus the `xkb::State`
//! that the compositor's `wl_keyboard.modifiers` events drive, and an
//! optional Compose state for dead keys. The rules that keep the hot path
//! allocation-free:
//!
//! - modifiers arrive as masks and go straight into `update_mask`; the
//!   compositor's mask is authoritative (see below), so `update_key` is
//!   never called and no per-key bookkeeping allocates.
//! - a key's text comes from `key_get_utf32` (no allocation, safe) for
//!   single-codepoint keys, and from [`raw`](super::raw) for composed
//!   ones; `key_get_utf8`'s heap `String` is never used.
//! - Caps/Num Lock are read through modifier *indices* cached at compile
//!   time: `mod_name_is_active` builds a `CString` per call, while
//!   `mod_index_is_active` is a bare C call.
//!
//! Why no `update_key`: the compositor computes its modifier mask from its
//! own XKB state, which already includes the pressed key, and hands it
//! over in the `modifiers` event the protocol guarantees after any
//! modifier-changing key. Calling `update_key` as well would apply the key
//! twice. The wiring is therefore one-directional: `modifiers` events in,
//! keysyms and text out.

use std::ffi::OsStr;
use std::fmt;

use xkbcommon::xkb::{self, STATE_MODS_LOCKED, State};

use super::keymap::KeymapSet;
use super::raw::{self, WriteOutcome};

/// evdev codes on the wire are XKB keycodes minus this: the protocol
/// carries the kernel number (`KEY_A` is 30) while XKB numbers from 8.
pub const EVDEV_OFFSET: u32 = 8;

/// The smallest buffer [`Keyboard::text_into`] is documented for: one
/// current key's UTF-8 (at most 6 bytes) plus a composed sequence. Shorter
/// buffers are legal and simply report [`TextError::Truncated`] sooner.
pub const MIN_TEXT_BYTES: usize = 64;

/// What producing one key's output can report. Cold paths only: the happy
/// path returns `Ok(len)` and allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    /// The key has no keysym in this state (a modifier, a bare dead key
    /// waiting for its partner, a key the layout leaves empty).
    NoSymbol,
    /// The text did not fit `buf`: the buffer's contents are unspecified.
    /// Bring at least [`MIN_TEXT_BYTES`]; composed text can exceed a tiny
    /// buffer honestly.
    Truncated,
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSymbol => write!(f, "the key has no keysym in this state"),
            Self::Truncated => write!(f, "the text did not fit the buffer"),
        }
    }
}

impl std::error::Error for TextError {}

/// One seat's decode state.
pub struct Keyboard {
    keymap: KeymapSet,
    state: State,
    compose: Option<Compose>,
    compose_locale: Option<std::ffi::OsString>,
}

struct Compose {
    table: xkb::compose::Table,
    state: xkb::compose::State,
}

impl Keyboard {
    /// Decode with `keymap` and no Compose: without a table every key
    /// decodes to its own keysym's text, and dead keys decode to nothing.
    /// For Compose, see [`Keyboard::with_compose`] (an explicit table,
    /// for tests) or [`Keyboard::set_compose_locale`] (the session
    /// locale, for apps).
    pub fn new(keymap: KeymapSet) -> Self {
        let state = State::new(keymap.keymap());
        Self {
            keymap,
            state,
            compose: None,
            compose_locale: None,
        }
    }

    /// Decode with `keymap`, feeding every press through `table` first.
    /// The table outlives the state that borrows it only through the C
    /// refcount (`compose::State::new` refs the table), but both are owned
    /// here anyway, state first so it drops first.
    pub fn with_compose(keymap: KeymapSet, table: xkb::compose::Table) -> Self {
        let state = State::new(keymap.keymap());
        let compose_state =
            xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
        Self {
            keymap,
            state,
            compose: Some(Compose {
                state: compose_state,
                table,
            }),
            compose_locale: None,
        }
    }

    /// Decode dead keys through a Compose table built for `locale`
    /// (explicit, never from the environment). Built on the keymap's own
    /// context and rebuilt on every keymap swap; a table that will not
    /// build (an interior NUL, a host without the data) degrades to
    /// direct decoding rather than failing the decode.
    pub fn set_compose_locale(&mut self, locale: &OsStr) {
        use std::os::unix::ffi::OsStrExt;
        // The locale is stored, not the table: the table belongs to a
        // keymap context, and a swap drops that context with the old map.
        if locale.as_bytes().contains(&0) {
            self.compose = None;
            self.compose_locale = None;
            return;
        }
        self.compose_locale = Some(locale.to_owned());
        self.compose = xkb::compose::Table::new_from_locale(
            self.keymap.context(),
            locale,
            xkb::compose::COMPILE_NO_FLAGS,
        )
        .ok()
        .map(|table| {
            let state = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
            Compose { state, table }
        });
    }

    /// Apply a `wl_keyboard.modifiers` event: depressed, latched, locked,
    /// and the effective group. The group is the protocol's effective
    /// layout index, so it lands in the locked-layout slot with the other
    /// two layout slots zero: effective layout ends up exactly `group`.
    pub fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        self.state
            .update_mask(depressed, latched, locked, 0, 0, group);
    }

    /// The single keysym for an evdev keycode in the current state, or
    /// `KEY_NoSymbol` when the key has none (or more than one).
    pub fn keysym(&self, evdev: u32) -> u32 {
        self.state.key_get_one_sym(xkb_keycode(evdev)).raw()
    }

    /// Write an evdev keycode's text into `buf`, returning the bytes
    /// written. Dead keys feed the Compose state when there is one: a key
    /// mid-sequence writes nothing yet (`Ok(0)`); a completed sequence
    /// writes the composed text; a key no sequence wants falls through to
    /// its own keysym. Without Compose, every key decodes to its own
    /// keysym's text. Keys with no keysym here (modifiers, unbound keys,
    /// a bare dead key) report [`TextError::NoSymbol`].
    ///
    /// No allocation after the keymap is compiled: single codepoints go
    /// through `key_get_utf32` plus `char::encode_utf8` into `buf`, and
    /// composed text through [`raw`](super::raw).
    pub fn text_into(&mut self, evdev: u32, buf: &mut [u8]) -> Result<usize, TextError> {
        let key = xkb_keycode(evdev);
        let keysym = self.state.key_get_one_sym(key);
        if keysym.raw() == xkb::keysyms::KEY_NoSymbol {
            return Err(TextError::NoSymbol);
        }
        if let Some(compose) = self.compose.as_mut() {
            match compose.state.feed(keysym) {
                xkb::compose::FeedResult::Accepted => match compose.state.status() {
                    // A lone key no sequence starts: `Accepted` with
                    // status `Nothing`. The key as itself, through the
                    // direct path -- the compose buffer holds nothing.
                    // (Only modifiers and the like report `Ignored`.)
                    xkb::compose::Status::Nothing => {}
                    xkb::compose::Status::Composed => {
                        let outcome =
                            raw::compose_utf8_into(compose.state.get_raw_ptr().cast(), buf);
                        compose.state.reset();
                        return Self::written(outcome);
                    }
                    xkb::compose::Status::Composing => return Ok(0),
                    xkb::compose::Status::Cancelled => {
                        // No sequence wants this pair: the pending dead
                        // key is swallowed (XCompose behavior) and the
                        // current key types as itself.
                        compose.state.reset();
                    }
                },
                xkb::compose::FeedResult::Ignored => {}
            }
        }
        direct_utf8(&self.state, key, buf)
    }

    /// The text of a repeat tick: the key as itself, without feeding
    /// Compose again (a held key that started a sequence keeps whatever
    /// the press fed; repeats of `repeated`-state keys never touch the
    /// in-progress sequence).
    pub fn repeat_text_into(&self, evdev: u32, buf: &mut [u8]) -> Result<usize, TextError> {
        direct_utf8(&self.state, xkb_keycode(evdev), buf)
    }

    fn written(outcome: WriteOutcome) -> Result<usize, TextError> {
        match outcome {
            WriteOutcome::Written(len) => Ok(len),
            WriteOutcome::Truncated => Err(TextError::Truncated),
        }
    }

    /// Caps Lock: the layout's `Lock` modifier, locked. A layout without
    /// one reports false.
    pub fn caps_lock(&self) -> bool {
        self.keymap.caps_index().is_some_and(|index| {
            self.state
                .mod_index_is_active(index, STATE_MODS_LOCKED)
        })
    }

    /// Num Lock: the layout's `Mod2` modifier, locked. Same absence rule.
    pub fn num_lock(&self) -> bool {
        self.keymap.num_index().is_some_and(|index| {
            self.state.mod_index_is_active(index, STATE_MODS_LOCKED)
        })
    }

    /// Whether this evdev keycode produces repeats.
    pub fn key_repeats(&self, evdev: u32) -> bool {
        self.keymap.key_repeats(xkb_keycode(evdev))
    }

    /// Swap in a new keymap (the `keymap` event mid-session), keeping the
    /// old one when the text refuses to compile: a client that goes dead
    /// on a bad keymap strands its app. The XKB state is rebuilt (nothing
    /// of the old layout survives) and any in-progress Compose sequence
    /// is dropped.
    pub fn replace_keymap(&mut self, text: String) -> Result<(), super::KeymapError> {
        let keymap = KeymapSet::compile(text)?;
        self.state = State::new(keymap.keymap());
        self.keymap = keymap;
        // A swap drops the old context with the old map, so a stored
        // locale rebuilds on the new one (degrading to direct decoding
        // when the table will not build); an explicit table stays, the
        // borrow being C-side only.
        if let Some(locale) = self.compose_locale.clone() {
            self.compose = None;
            self.set_compose_locale(&locale);
        } else if let Some(compose) = self.compose.as_mut() {
            compose.state =
                xkb::compose::State::new(&compose.table, xkb::compose::STATE_NO_FLAGS);
        }
        Ok(())
    }

    /// Drop any in-progress Compose sequence (focus loss).
    pub fn reset_compose(&mut self) {
        if let Some(compose) = self.compose.as_mut() {
            compose.state.reset();
        }
    }
}

/// One key's own text, no Compose: `key_get_utf32` is 0 for keys without
/// exactly one codepoint (modifiers, multi-keysym keys, dead keys), and
/// `char::encode_utf8` into the caller's buffer allocates nothing. A
/// buffer too small for the character reports [`TextError::Truncated`],
/// never a partial character. Codepoint 0 is not text (it is what the C
/// side reports for "no codepoint", and `char::from_u32` would happily
/// make it U+0000): [`TextError::NoSymbol`.
fn direct_utf8(state: &State, key: xkb::Keycode, buf: &mut [u8]) -> Result<usize, TextError> {
    let codepoint = state.key_get_utf32(key);
    let ch = char::from_u32(codepoint).ok_or(TextError::NoSymbol)?;
    if ch == '\0' {
        return Err(TextError::NoSymbol);
    }
    let mut scratch = [0u8; 4];
    let text = ch.encode_utf8(&mut scratch);
    if text.len() > buf.len() {
        return Err(TextError::Truncated);
    }
    buf[..text.len()].copy_from_slice(text.as_bytes());
    Ok(text.len())
}

fn xkb_keycode(evdev: u32) -> xkb::Keycode {
    // XKB numbers from 8; the wire carries evdev. Saturating: callers pass
    // real evdev codes, and a wild one must not wrap into a valid key.
    xkb::Keycode::new(evdev.saturating_add(EVDEV_OFFSET))
}

/// A Compose table from an explicit locale (no environment read: the
/// caller decides, so tests pin `"C"` and apps pass the session locale).
/// Interior-NUL locales are refused rather than reaching the crate's
/// `CString::new(...).unwrap()`.
pub fn compose_table_from_locale(
    keymap: &KeymapSet,
    locale: &OsStr,
) -> Result<xkb::compose::Table, super::KeymapError> {
    use std::os::unix::ffi::OsStrExt;
    if locale.as_bytes().contains(&0) {
        return Err(super::KeymapError::Compile);
    }
    xkb::compose::Table::new_from_locale(
        keymap.context(),
        locale,
        xkb::compose::COMPILE_NO_FLAGS,
    )
    .map_err(|()| super::KeymapError::Compile)
}
