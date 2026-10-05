---
title: "Binds do not repeat while held; no key repeat at all"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Binds do not repeat while held; no key repeat at all

Filed 2026-10-05 from the `desktop-keys` verification (docs/backlog/packaging/desktop-keys.md:
"whether a held key repeats a bind (holding volume up must step, not fire
once)"). Serves **daily-drive**: holding volume-up stepping once per
press is not daily-drivable, and neither is a terminal with no key
repeat.

## The gap

Holding a bound key fires its bind exactly once. Two layers agree:

- Smithay (pinned fork rev `035d447`, `src/input/keyboard/mod.rs`
  `key_input` + `input_from_source`): a press of a keycode already
  held by the same source is absorbed before the filter -- "don't
  double-run the filter (avoids re-triggering shortcuts)". A kernel
  repeat arriving as another press from the same device never reaches
  scoot's bind filter.
- scoot runs no repeat timer of its own: no `repeat_rate` /
  `repeat_delay` configuration, no timer re-firing binds, and nothing
  sending repeat to clients either (`grep repeat_rate
  crates/scoot/src` is empty outside pixman/upscale and prose).

So `XF86AudioRaiseVolume` held steps once, and (same root cause) a
held key in a terminal does not repeat. niri repeats binds by
default with per-bind `repeat=false`; Hyprland has a `repeating`
flag. scoot has neither.

## What to do

- A repeat timer on the compositor's key path: while a bound key is
  held past the delay, re-fire its bind at the rate (and, separately,
  forward repeat to the focused client per the keymap's repeat
  info). Per-bind opt-out (a volume step wants repeat; `close` or
  `quit` must never repeat).
- Pin: hold fires N>1 times, release stops it, unbound holds stay
  silent, opt-out binds fire once. Live proof on `--tty` (hold a Fn
  key, count steps) since that is the path that matters.
- The `desktop-keys` docs (`docs/nix.md` "Hardware keys") state the
  current once-per-press behavior until this lands; update them
  there.

## Not in this ticket

The `allow-when-locked` per-bind flag (sibling ticket
`bind-allow-when-locked`); per-device repeat rates; anything about
which keys the keymap binds (that is `desktop-keys`, landed).
