---
title: "`scootui` widgets: text entry with password mode, virtualized list, focus chain"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-keyboard"
milestone: "M7"
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
