---
title: "Look leaves for foot, Helix, btop and starship"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Look leaves for foot, Helix, btop and starship

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

foot, starship, Helix and btop are static example files linked from the
store (`theme-home.nix:657-678`).

## What to do

- foot: `[colors-dark]` or `[colors-light]` generated from `[terminal]`
  (foot 1.28 has no `[colors]`), `alpha` from `opacity.terminal`; new
  terminals only. Research only: a live OSC push into running ptys (it can
  interleave with a program's own output).
- Helix and btop: the look's static leaves (trusted looks), then `SIGUSR1`
  to the user's own `hx` and `SIGUSR2` to btop (their documented reloads),
  each opt-out. starship: next prompt.
- These leaves are dropped from installed looks without `--trust` (foot
  `shell`, starship custom commands).

## Evidence to record

A swap observed in a running Helix and btop and a new foot; the trust drop
tested.

## Not in this ticket

GTK/Qt.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
