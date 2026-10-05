---
title: "No per-bind allow-when-locked: volume and brightness die at the lock screen"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# No per-bind allow-when-locked: volume and brightness die at the lock screen

Filed 2026-10-05 from the `desktop-keys` verification
(docs/backlog/resolved/desktop-keys-done.md: "whether `[binds]` fire
while the session is locked (volume/brightness should,
launcher/clipboard must not)"). Serves **daily-drive**: volume and
brightness keys going dead the moment the session locks is the kind
of papercut that sends a user back to their old compositor.

## The gap

While the session is locked, no `[binds]` action fires except VT
switching (`crates/scoot/src/compositor/input.rs::key`, gated on
`session_lock.is_locked()`; backstopped in `State::act`; pinned by
`an_action_keybinding_does_not_fire_while_locked`). That gate is
correct for `spawn` in general -- a terminal from behind the lock
screen would be a complete bypass -- but it cannot tell a harmless
`spawn` (volume, brightness, media) from a bypass (terminal,
launcher, clipboard). So the `desktop-keys` keymap's hardware binds
go to the locker as ordinary keystrokes while locked. niri solves
exactly this with `allow-when-locked=true` on `spawn` binds (its
default config marks the volume binds that way); Hyprland has a
`locked` bind flag.

## What to do

- A per-bind `allow-when-locked` flag for `spawn` binds only (never
  for layout/focus/close/quit actions, which keep today's refusal):
  config syntax, the gate in `input.rs`, the `act` backstop's
  position on an allowed spawn, and tests that fail before (an
  allowed volume spawn fires while locked and reaches no window; a
  terminal spawn from behind the lock still does not).
- The `desktop-keys` keymap then marks exactly the volume,
  brightness and media binds allowed (launcher, clipboard, lock and
  capture stays refused), and the `docs/nix.md` "Hardware keys"
  limitation notes go away.
- Decide the IPC shape too: `Request::Action` is refused while
  locked today -- whether an allowed-spawn action through IPC stays
  refused (it bypasses focus; the keystroke path does not) needs a
  deliberate answer, not an accident.

## Not in this ticket

Bind repeat (sibling ticket `bind-repeat`); which keys the keymap
binds (that is `desktop-keys`, landed).
