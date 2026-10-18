---
title: "Spike: the keyboard stack for typed text (kbvm vs libxkbcommon), measured"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-17"
---

# Spike: the keyboard stack for typed text

Filed 2026-10-12 from the [shell-apps design](../../../../dev/design/shell-apps.md#keyboard-and-text-entry).
Serves **daily-drive** and **computer use**. A launcher or greeter that types the wrong
character into a password field locks the user out, and agents type through `scoot msg
type`, which reaches the client through the same keymap. Unblocked: it is research, and it
gates the launcher and greeter prototypes.

## The gap

Nothing in the shell family reads a keymap. scootbar drops the `wl_keyboard.keymap` fd unread
and matches raw evdev codes (`crates/scootbar/src/daemon/popup/events.rs:30-38,378`).

## What to do

Throwaway binaries, removed after the run, with a `dev/spikes/` record as the result (as
the [D-Bus spike](../../../../dev/spikes/scootbar/dbus-client.md) did):

- one on `kbvm` (pure Rust, MIT OR Apache-2.0);
- one on `xkbcommon` (the crate over libxkbcommon).

Each binds `wl_seat` v9+, reads the keymap scoot sends, and turns key events into text and keysyms.

Measure on the dev VM under `scoot --headless`, and on the M2:

- correctness on `us`, `us(intl)` dead keys, `de`, `fr` (AZERTY, shifted digits), `dvorak`,
  a two-group `us,ru` with `grp:alt_shift_toggle`, and Compose sequences (`XCompose`),
  including whether Compose depends on the locale;
- keymap compile time;
- stripped size and `.text` added, and RSS added;
- whether either needs `unsafe` in the consumer;
- **how each turns a keysym into UTF-8**: the widget needs a call that writes into a
  caller-supplied buffer, because a `String`-returning call leaves copies of typed password
  characters in freed heap;
- whether `memfd_secret` is enabled on the M2 and dev VM kernels (scootgreet's locked page).

## Acceptance

A decision recorded in `dev/spikes/scootui-keyboard.md` with the table, and the
[design](../../../../dev/design/shell-apps.md#keyboard-and-text-entry) updated to the pick.

## Evidence the review needs

- The per-layout pass/fail table, typed through `scoot msg type` and through real keys on the M2.
- Size and RSS rows with the SHA.
- The spike binaries' source in the PR, deleted in the same PR after measuring.

## Not in this ticket

The widget (`scootui-entry-and-list`); IME (`text-input-v3`, a later follow-up).

## Resolution

Resolved 2026-10-17. Decision: `xkbcommon`, no wrapper crate. Record with
the tables: [dev/spikes/scootui-keyboard.md](../../../../dev/spikes/scootui-keyboard.md);
the [design](../../../../dev/design/shell-apps.md#keyboard-and-text-entry)
updated to the pick in the same branch. Spike source measured at
`c96e7e341`, deleted before merge. What did not run: real keys on M2
hardware, the M2 Asahi kernel's `memfd_secret` (dev VM has it). Found on
the way: the shared keymap-fd offset bug, filed as
[core/keymap-fd-shared-offset](../../../backlog/core/keymap-fd-shared-offset.md).
