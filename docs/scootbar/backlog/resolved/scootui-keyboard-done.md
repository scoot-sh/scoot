---
title: "`scootui` keyboard: keymap, modifiers, repeat, keysym to a caller's buffer"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
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

## Resolution

Shipped behind `scootui`'s `keyboard` feature (`crates/scootui/src/keyboard/`):
`negotiate_version` binds 9, or 10 where offered; the keymap fd is read
with a 1 MiB cap (seek-to-0 for the shared file offset, empties and
oversize refused before buffering, hostile/UTF-8/truncated inputs all
loud); modifiers drive the XKB state with Caps (`Lock`) and Num Lock
(`Mod2`) indices cached at compile; one timerfd arms only while a
repeatable key holds (rate 0 never arms, release/focus-loss/keymap-swap
disarm, latest press wins); keysyms and UTF-8 go into a caller-supplied
buffer (single codepoints via `key_get_utf32`, composed text via one
`unsafe` wrapper in `keyboard::raw` -- the crate moved from
`forbid(unsafe_code)` to `deny(unsafe_code)` with that module allowed, a
separate crate rejected as needless surface); Compose via an explicit
locale table (system `"C"` for apps, an embedded two-sequence buffer in
tests), degrading to direct decoding when the table will not build.

Acceptance, all green on the dev VM (aarch64, rustc 1.97.1): 35 unit
tests (us/de/fr/dvorak/us,ru groups, us(intl) dead keys and compose,
evdev offset, hostile keymaps, shared-offset rewind, repeat transitions,
caps/num lock incl. Shift-cancels-Caps, zero-alloc per event via
`scootbg_mem::count_allocations`, wire KeyState discriminants) plus 2
headless tests -- the spike's layout set (`AbC xyz 123!?`, `café`,
`yYzZß?`, `aqz&1`, `oea.`) typed through `scoot msg type` and a
virtual-keyboard hold (ticks while held, silence after release, silence
and disarm after focus loss, <=8 voluntary switches in 3 s idle vs ~75
for a 40 ms timer). Three pins mutation-proved (seek, evdev offset,
rate-0 arm). No fuzz target: the keymap-text path is three small pure
functions fully covered by hostile unit tests; libFuzzer would mostly
exercise libxkbcommon's C parser.

Link: `ldd`/`NEEDED` show exactly `libxkbcommon.so.0` beyond std libs
for a keyboard binary; `cargo tree -p scootbar` has no xkbcommon and
the release `scootbar` is byte-identical to main (`b757a784…`, 3019520
bytes, identical `NEEDED`). Missing library fails at exec with one
loader line, exit 127 (`dev/scootui-keyboard-link.md`). Docs in the
same PR: rustdoc on the module, the link note, CI loops extended.
