//! The seat keyboard: keymap, modifiers, repeat, keysym and Compose text.
//!
//! One `KeyboardSeat` per seat decodes `wl_keyboard` into keysyms and
//! UTF-8 written into a caller-supplied buffer. Behind the `keyboard`
//! feature, which links libxkbcommon (below). Everything else in scootui
//! -- and in particular scootbar, which binds the seat only to drop the
//! keymap unread -- pays nothing for it.
//!
//! ## Protocol
//!
//! `wl_seat` binds at 9, or 10 where offered ([`negotiate_version`]).
//! Handlers take plain data rather than Wayland objects, so the whole
//! layer below the [`dispatch_keyboard`] mapping is unit-testable without
//! a compositor:
//!
//! - `keymap`: the fd is read with a [`size cap`](MAX_KEYMAP_BYTES)
//!   ([`read_keymap_bytes`]), seeking to 0 first: every client's fd is the
//!   same open file description, so a plain read after another client's
//!   read gets zero bytes (the core ticket
//!   `docs/backlog/core/keymap-fd-shared-offset.md` fixes this
//!   server-side; the seek covers clients until then). A replacement
//!   keymap mid-session swaps in; a refusal keeps the old one.
//! - `modifiers`: depressed, latched, locked and the effective group go
//!   straight into the XKB state. Caps Lock and Num Lock read out through
//!   [`KeyboardSeat::caps_lock`] / [`KeyboardSeat::num_lock`].
//! - `key`: evdev codes (XKB keycode minus 8) decode to one keysym and
//!   UTF-8 into the caller's buffer, never a returned `String`. Without a
//!   Compose table every key decodes to its own keysym; with one, dead
//!   keys compose (`café` from `'`-then-`e` on `us(intl)`) and a cancelled
//!   sequence types the current key as itself.
//! - `repeat_info`: the rate and delay arm one [`timerfd`](Repeater),
//!   created on press and dropped on release, focus loss or keymap change.
//!   Rate 0 arms nothing; on a version-10 seat the compositor may then
//!   send `repeated`-state keys, which decode as ticks.
//!
//! No allocation per key event after the keymap is compiled: the text
//! lands in the caller's buffer through `key_get_utf32` (single
//! codepoints) or the one `unsafe` module ([`raw`]) for composed text,
//! and the lock-key queries use indices cached at compile time. The unit
//! tests pin this with `scootbg_mem::count_allocations`.
//!
//! ## Linking
//!
//! The `xkbcommon` crate links libxkbcommon dynamically
//! (`libxkbcommon.so.0` in `ldd`/`NEEDED`): the reference implementation
//! every compositor and toolkit uses, already resident -- and therefore
//! zero marginal closure -- on a scoot desktop, where the compositor
//! links it too. It is the one C library in the shell family's link set;
//! `scootbar` (which never enables this feature) links exactly what it did
//! before, byte-identically (the ticket's CI evidence). A host without
//! the library refuses loudly at exec: the dynamic loader prints one
//! `libxkbcommon.so.0: cannot open shared object file` line and exits
//! 127. No fallback, no retry loop: keyboard text cannot work without the
//! decoder, and crashing in a supervisor loop would wedge the session, so
//! the failure stays a single loud refusal. A future `scootnotify`,
//! `scootlaunch` or `scootgreet` package carries libxkbcommon in its Nix
//! `buildInputs` (RUNPATH then finds it); building and testing find it
//! through the dev shell's compositor dependencies, which already include
//! it. See `dev/scootui-keyboard-link.md`.

mod keymap;
mod raw;
mod repeat;
mod seat;
mod state;
#[cfg(test)]
mod tests;

pub use keymap::{KeymapError, KeymapSet, MAX_KEYMAP_BYTES, read_keymap_bytes};
pub use repeat::Repeater;
pub use seat::{KEYMAP_FORMAT_XKB_V1, KeyOutput, KeyState, KeyboardSeat, MAX_SEAT_VERSION,
    MIN_SEAT_VERSION, negotiate_version};
pub use state::{EVDEV_OFFSET, MIN_TEXT_BYTES, TextError, compose_table_from_locale, Keyboard};

use std::os::unix::io::{AsFd, OwnedFd};

/// Map one `wl_keyboard` event onto a [`KeyboardSeat`]. The app's
/// `Dispatch<WlKeyboard>` impl is three lines: forward the event, the seat
/// and a reusable text buffer; the returned [`KeyOutput`] (if any) carries
/// `text_len` bytes of UTF-8 at the buffer's start. The buffer should hold
/// at least [`MIN_TEXT_BYTES`]; shorter buffers truncate (never
/// partially: [`TextError::Truncated`] collapses to empty output).
pub fn dispatch_keyboard(
    seat: &mut KeyboardSeat,
    event: wayland_client::protocol::wl_keyboard::Event,
    text: &mut [u8],
) -> Result<Option<KeyOutput>, KeymapError> {
    use wayland_client::protocol::wl_keyboard::Event as Kb;
    match event {
        Kb::Keymap { format, fd, .. } => {
            // `fd` arrives owned; the handlers borrow it and it closes
            // here, on return.
            let owned: OwnedFd = fd;
            seat.keymap_event(format.into(), owned.as_fd())
                .map(|()| None)
        }
        Kb::Enter { .. } => {
            seat.enter();
            Ok(None)
        }
        Kb::Leave { .. } => {
            seat.leave();
            Ok(None)
        }
        Kb::Key { key, state, .. } => Ok(seat.key_event(
            key,
            KeyState::from_wire(state.into()).unwrap_or(KeyState::Released),
            text,
        )),
        Kb::Modifiers {
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
            ..
        } => {
            seat.modifiers_event(mods_depressed, mods_latched, mods_locked, group);
            Ok(None)
        }
        Kb::RepeatInfo { rate, delay } => {
            seat.repeat_info_event(rate, delay);
            Ok(None)
        }
        _ => Ok(None),
    }
}
