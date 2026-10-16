---
title: "Look leaves for foot, Helix, btop and starship"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-16"
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

## Resolution (2026-10-16, PR TBD)

foot shipped in #633. This ticket landed Helix, btop and starship:

- Static leaves (`apps/helix.toml`, `apps/btop.theme`,
  `apps/starship.toml`) copy verbatim from the source look into the
  render, inherited through `extends`, shape-checked; shipped looks
  carry them (ginger-night, moonrise, vinyl-sunset all three;
  music-desk helix + starship; radial-burst none).
- `SIGUSR1` to the user's own `hx` and `SIGUSR2` to btop (both
  verified against the pinned sources and live against the real
  binaries, macOS and Linux), each opt-out (`--no-helix`,
  `--no-btop`); starship follows on the next prompt.
- Installed looks drop the leaves without `--trust` (which prints
  them first); `RENDER_VERSION` 9; `install.sh` fetches the trio
  best-effort.
- Evidence: live theme swaps observed in running hx and btop on
  macOS and Linux (pty captures, color counts before/after), the
  trust drop tested at unit, CLI and binary level. Full report in
  the scratchpad (`W5-look-leaves-terminal-rest-report.md`).

## Not in this ticket

GTK/Qt.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
