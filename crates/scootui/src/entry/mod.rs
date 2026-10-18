//! Text entry, virtualized list, inline selector and focus chain: the
//! input-heavy widgets `scootlaunch` and `scootgreet` type and pick
//! through, drawn in the same tokens as the bar.
//!
//! Pure state machines plus paint into the caller's canvas: no Wayland
//! objects and no I/O, so every edit, scroll and golden is a unit test.
//! Behind the `entry` feature, which `scootbar` never enables.
//!
//! ## The minimal API, per consumer
//!
//! `scootlaunch`'s launch-ui needs a query entry plus a ranked list whose
//! rows carry match-highlight spans from its own matcher:
//! [`Entry`] (with [`Entry::apply_key`] fed from the seat's decoded keys)
//! plus [`List`] (with [`List::set_highlights`] fed from the matcher and
//! [`paint_list`](crate::entry::paint_list) drawing only
//! [`List::visible_range`]).
//!
//! `scootgreet` needs a password entry plus user and session selectors
//! across one focus chain: [`PasswordEntry`] (masked, into a
//! caller-supplied never-resized buffer, refusing copy, paste and IME),
//! two [`Selector`]s, and [`FocusChain`] (Tab/Shift+Tab with an accent
//! focus ring). A future `scootlock` needs exactly the entry, focus and
//! lock-key pieces, so this set is built once for both.
//!
//! Picking a row yields an [`Activate`] carrying an **owned** id: the
//! popup's [`Activate`](crate::popup::Activate) keeps its `&'static str`
//! (the bar's modules are compiled in; changing it would rebuild every
//! module for nothing), while these widgets name rows the caller's data
//! owns (desktop file ids, usernames, session ids), so the id is a
//! `String` taken at activation time.
//!
//! ## Allocation
//!
//! Nothing here allocates on the content path after warm-up: the entry
//! owns one buffer sized once to its cap (the password entry owns none
//! at all, writing into the caller's slice), and the list owns row
//! indices and one reused highlight scratch, never row text. The unit
//! tests pin this with `scootbg_mem::count_allocations` (the crate's lib
//! test target installs the counting allocator): zero per keystroke and
//! zero per scroll after the first fill.
//!
//! ## No `unsafe`
//!
//! This module is safe Rust throughout: no `unwrap`, no `expect`, no
//! `panic!`, no indexing past what a length check allowed. Every fallible
//! operation reports through `Option` or a small outcome enum.

mod line;
mod focus;
mod list;
mod paint;

#[cfg(test)]
mod goldens;
#[cfg(test)]
mod tests;

pub use line::{
    Entry, InsertOutcome, KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT,
    KEY_PAGE_DOWN, KEY_PAGE_UP, KEY_RIGHT, KEY_TAB, KEY_UP, MASK_CHAR, MAX_ENTRY_BYTES,
    PasswordEntry,
};
pub use focus::FocusChain;
pub use list::{Activate, Highlight, List, MAX_HIGHLIGHT_ROWS, MAX_SPANS_PER_ROW, Selector};
pub use paint::{Geometry, paint_entry, paint_list, paint_password, paint_selector};
