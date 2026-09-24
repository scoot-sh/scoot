---
title: "Rounded clip and focus ring follow the layout slot, not the client's committed size: a client that doesn't fill its slot shows mismatched corners (gh #205, reopened)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Clip and ring the committed size; tell clients they're tiled

Filed 2026-09-24 from a live re-verification of gh #205 on `main` `1f2fe5c`
(scale 1.5, `corner_radius = 10`, `focus_ring_width = 4`). Serves
**daily-drive** (the default look of the default terminal) and
**computer use** (window rects that match what is drawn).

## What is wrong

foot (default `resize-by-cells=yes`) commits a buffer rounded down to whole
cells: 1437x1602 physical in a 1449x1625 slot. scoot's rounded clip and ring
use the layout slot (`clip_rect(placement)`), so the ring rounds a corner
the content never reaches (a green gap between square content and a curved
ring) at the right and bottom corners. Not fractional-specific (reproduces at
1.0). With `resize-by-cells=no` all four corners pass a per-pixel check, so
PR #207's fix itself holds.

foot's CHANGELOG says cell-rounding applies to *floating* windows; scoot
sends no `xdg_toplevel` tiled states (`tiled_left/right/top/bottom`, v2+).

## What to do (both, likely)

1. Send tiled states for windows in the scrolling layout (not for
   fullscreen, which has its own state; check what floating means for
   scoot, which has none today). Measure that foot, GTK, Qt then fill the
   slot exactly.
2. Clip and ring what the client actually committed (the window geometry
   of its last commit, clamped to the slot, centred/aligned per layout
   rules), so any client that still under-fills (e.g. a fixed-size dialog,
   an old client) gets a matching ring. Decide how the ring and rounded clip
   follow a mid-resize commit without flicker.

## Evidence

Repro artifacts: scratchpad `r205/` (the coordinator has the paths) and
`check_corners.py` (a per-corner pixel check that can become a harness
test). Fail-first harness test with a client committing a buffer smaller
than its slot; live foot defaults at 1.0 and 1.5 on headless and `--tty`,
all four corners passing; screenshot for the issue.
