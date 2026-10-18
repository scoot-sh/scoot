---
title: "`scootui` widgets: text entry with password mode, virtualized list, focus chain"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# `scootui` entry, list and focus

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#keyboard-and-text-entry)
and [scootgreet's needs](../../../dev/design/scootgreet.md#architecture-and-what-the-shared-crate-must-offer).
Serves **both**.

## What to do

- **Text entry.** One line, with a byte cap the caller sets; caret and word moves,
  BackSpace/Delete, Ctrl+W/U/A; paste (non-password only).
  - **Password mode** is masked, writes into a buffer the caller supplies (scootgreet's is
    locked memory) and never resizes it, and refuses copy and IME.
- **A virtualized list.** Unbounded rows, drawing only the visible ones (the popup's
  `MAX_WIDGETS = 16` stays for the bar), with selection, scroll and match-highlight spans.
- **An inline selector and a focus chain** (Tab/Shift+Tab), with a focus ring in `accent`.
- **Activation carries an owned id**, not `&'static str` (`popup/mod.rs:215`).

## Acceptance

- Zero allocation per keystroke and per scroll after warm-up (`count_allocations`).
- A test that reads the password buffer after clear and finds zeros.
- Snapshot goldens for each widget state.
- A 65,536-row list scrolls with a per-frame cost independent of the row count.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Any app's screen.

## Resolution

Shipped behind a new `entry` feature on `scootui`
(`crates/scootui/src/entry/`): pure state machines plus paint into the
caller's canvas, no Wayland, no I/O, no new dependency. `scootbar`
pins `scootui` at `default-features = false` plus `icons, wayland,
control`, so it never enables `entry`: `cargo tree -p scootbar` shows
no new edge and the release binary is byte-identical to main's
(`b757a784…`, 3019520 bytes stripped, identical `NEEDED`
`libgcc_s/libm/libc/ld-linux`, `cmp` clean). The popup's `Activate`
keeps its `&'static str` (the bar's modules are compiled in); these
widgets carry their own `entry::Activate` with an owned `String` id,
taken at activation time, for rows the caller's data owns.

`Entry`: one line, caller-set byte cap (clamped to `MAX_ENTRY_BYTES =
8192`), UTF-8 caret by character, word moves over alphanumeric/`_`
runs, BackSpace/Delete, Ctrl+W (word plus preceding whitespace),
Ctrl+U, Ctrl+A (select-all only, replacing on next edit), paste;
`apply_key` maps the documented keysym table (BackSpace/Delete/arrows/
Home/End, Ctrl+arrows/letters by keysym or control code) and inserts
other printable text, returning whether the key was consumed.
`PasswordEntry` edits over a caller-supplied `&mut [u8]` that is never
resized (a slice: growing it cannot compile), draws one mask glyph per
character, and has no paste method, returns `None` from `copy_text`
and `false` from `accepts_ime`; `clear` and `Drop` zero the whole
slice, deletes zero the freed tail at once. `List` owns three scalars
plus one reused highlight scratch (128 rows, 8 spans each, both capped)
and never row text: selection wraps, pages move by the visible count,
`visible_range` is scalar arithmetic, `set_highlights` refills owned
memory; `apply_key` covers arrows/PageUp/PageDown/Home/End (Ctrl too)
and Ctrl+P/Ctrl+N. `Selector` is a collapsed choice line plus an
expanded `List` in the same surface; `FocusChain` cycles Tab
(`tab_next`) and Shift+Tab (`shift_tab_prev`) with wraparound.
Paint maps onto the bar tokens: text `foreground`, placeholder `dim`,
caret and 2 px focus ring `accent`, selection and selected row `accent`
fill with `background` ink, match spans `hover` (`urgent` on the
selected row, where `hover` would vanish on the `accent` fill).

Acceptance, all green on the dev VM (aarch64, rustc 1.97.1):
`count_allocations == 0` per keystroke (entry and password) and per
scroll plus highlight refill after warm-up, each mutation-proved
(`format!` injected into `insert_text`, both password and plain, and
into `select_next`: all three pins go red, green again on revert); a
test reading the whole caller slice after `clear` (and after scope-end
drop) finds zeros; nine snapshot goldens (empty/text/selected/
unfocused entry, masked password, highlighted list, scrolled list,
collapsed/expanded selector) as FNV-1a canvas hashes plus targeted
pixel asserts; a 65,536-row frame (move, window, ten visible rows plus
span reads) costs 1198.1 ns mean against 1179.2 ns at 64 rows (3
alternating rounds of 5,000 frames; per-round rows 1402.5/1357.0,
1387.5/860.1, 747.7/1377.2 ns), asserted under a 1 ms budget and within
5x of the small list; a paint test counts the row callback: exactly
the ten visible rows out of 65,536. Hostile input pinned (100 KiB
paste, emoji at the cap, zero cap, `usize::MAX` scroll, 1,000-row and
20-span highlight overflow, inverted/past-end/`u32::MAX` spans,
zero-size geometry). No `unwrap`/`expect`/`panic!`/indexing past a
checked bound in the library; no fuzz target added (no parser surface:
edits are `&str`-in/`&str`-out state machines with hostile unit
coverage, and all five fuzz workspaces still `cargo check --locked`
clean). No user-facing surface changes, so no site/README delta is
owed: nothing user-facing changed (no shipped binary enables the
feature). Unblocks `launch-ui`, `greet-ui`, `extract-scootui` (their
`blocked` fields cleared in this PR).
