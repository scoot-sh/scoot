//! The crate's only `unsafe`: one libxkbcommon call that writes UTF-8
//! into a caller-supplied buffer.
//!
//! Everything else in `keyboard` goes through the safe `xkbcommon` crate
//! wrappers, which keep `forbid(unsafe_code)`-style consumer code: the
//! spike proved the full client pattern compiles without `unsafe`
//! (`dev/spikes/scootui-keyboard.md`). One call has no safe form that
//! meets the ticket: `compose::State::utf8` builds a heap `String` per
//! call, which breaks both halves of the text contract (into a
//! caller-supplied buffer, never a returned `String`; no allocation per
//! key event after the keymap is compiled). The C function underneath
//! takes `(buffer, size)` and returns a `snprintf`-like count, so this
//! wrapper calls it directly. Single keys need no wrapper:
//! `key_get_utf32` is already safe and allocation-free, and a key with
//! more than one codepoint outside Compose is pathological (reported as
//! [`TextError::NoSymbol`](super::state::TextError)).
//!
//! The `allow` is scoped to this module only: the crate is
//! `#![deny(unsafe_code)]`, and nothing else in it may use `unsafe`.

#![allow(unsafe_code)]

use std::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
    fn xkb_compose_state_get_utf8(state: *mut c_void, buffer: *mut c_char, size: usize) -> c_int;
}

/// The outcome of one caller-buffer write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WriteOutcome {
    /// `len` bytes of valid UTF-8 now sit in the buffer's first `len`
    /// bytes.
    Written(usize),
    /// The text did not fit: the buffer's contents are unspecified. The
    /// C call returns the full length it wanted, so the caller knows how
    /// much room to bring; nothing is read past the buffer either way.
    Truncated,
}

/// Write the composed text into `buffer`.
///
/// # Safety (the caller's two obligations)
///
/// - `state` is the live `get_raw_ptr()` of a `compose::State` whose
///   status is `Composed`, and that outlives the call (the C function
///   reads the state and writes only `buffer`).
/// - `buffer` is writable for its whole length. An empty buffer always
///   reports [`WriteOutcome::Truncated`] for non-empty text.
pub(super) fn compose_utf8_into(state: *mut c_void, buffer: &mut [u8]) -> WriteOutcome {
    // SAFETY: the caller guarantees `state` is a live compose-state
    // pointer and `buffer` is wholly writable. The C function writes at
    // most `buffer.len()` bytes (snprintf semantics) and touches nothing
    // else.
    let wanted = unsafe {
        xkb_compose_state_get_utf8(state, buffer.as_mut_ptr().cast::<c_char>(), buffer.len())
    };
    // Negative is the C side's unspecified error; `wanted >= room` (with
    // `room == 0` included) means the NUL-terminated write did not fit.
    // `wanted` counts text bytes without the terminator, so `wanted <
    // room` leaves the buffer holding exactly `wanted` bytes of UTF-8:
    // libxkbcommon never emits a partial character.
    if wanted < 0 || (wanted as usize) >= buffer.len() {
        WriteOutcome::Truncated
    } else {
        WriteOutcome::Written(wanted as usize)
    }
}
