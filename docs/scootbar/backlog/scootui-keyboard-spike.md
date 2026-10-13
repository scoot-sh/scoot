---
title: "Spike: the keyboard stack for typed text (kbvm vs libxkbcommon), measured"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# Spike: the keyboard stack for typed text

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#keyboard-and-text-entry).
Serves **daily-drive** (a launcher or greeter that types the wrong character into a
password field locks the user out) and **computer use** (agents type through
`scoot msg type`, which reaches the client through the same keymap).

## The gap

Nothing in the shell family reads a keymap: scootbar drops the `wl_keyboard.keymap` fd
unread and matches raw evdev codes (`crates/scootbar/src/daemon/popup/events.rs:30-38,378`).
scootlaunch and scootgreet need real text from any layout.

## What to do

Throwaway binaries (removed after; this entry and a `dev/spikes/` record are the result,
as the [D-Bus spike](../../../dev/spikes/scootbar/dbus-client.md) did): one on `kbvm`
(pure Rust, MIT OR Apache-2.0), one on `xkbcommon` (the crate over libxkbcommon). Each binds
`wl_seat` v9+, reads the keymap scoot sends, and turns key events into text and keysyms.

Measure, on the dev VM under `scoot --headless` and on the M2:
correctness on `us`, `us(intl)` dead keys, `de`, `fr` (AZERTY, shifted digits), `dvorak`,
a two-group `us,ru` with `grp:alt_shift_toggle`, and Compose sequences (`XCompose`);
keymap compile time; stripped size and `.text` added; RSS added; whether either
needs `unsafe` in the consumer.

## Acceptance

A decision recorded in `dev/spikes/scootui-keyboard.md` with the table, and the
[design](../../../dev/design/shell-apps.md#keyboard-and-text-entry) updated to the pick. Prior to confirm
or overturn: `kbvm` if it passes the layout set (dead keys included) and costs under
300 KB; otherwise `xkbcommon` behind a small wrapper crate holding the `unsafe`.

## Evidence the review needs

The per-layout pass/fail table typed through `scoot msg type` and through real keys
on the M2, size and RSS rows with the SHA, and the spike's binaries' source in the PR
(deleted in the same PR after measuring, as the D-Bus spike did).

## Not in this ticket

The widget (`scootui-entry-and-list`), IME (`text-input-v3`, a later follow-up).
