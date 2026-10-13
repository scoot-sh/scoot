---
title: "`scootui` keyboard: keymap, modifiers, repeat, keysym to a caller's buffer"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-keyboard-spike, scootui-extract-wayland"
milestone: "M7"
---

# `scootui` keyboard

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#keyboard-and-text-entry).
Serves **both**: typed text for the launcher and greeter, and agents type through it.

## What to do

Build the spike's pick behind a `keyboard` feature:

- `wl_seat` bound at v9+, and v10 where offered;
- the keymap fd read with a size cap;
- modifiers, with Caps Lock and Num Lock exposed;
- key repeat on one `timerfd`, armed only while a repeatable key is held (rate 0 means none);
- keysyms, and UTF-8 written **into a caller-supplied buffer** (never a returned `String`);
- Compose, if the spike says it is covered.

## Acceptance

- Zero idle wakeups with the feature on and no key held.
- The spike's layout set typed correctly through `scoot msg type` in a headless test.
- A repeat test: hold, release, and focus loss mid-repeat.
- No allocation per key event after the keymap is compiled.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The entry widget, IME.
