---
title: "CSD backdrop is cut off at the ring strip's height: background sliver beside a client rounder than the ring"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-03"
---

# CSD backdrop is cut off at the ring strip's height: background sliver beside a client rounder than the ring

Filed 2026-11-03 from writing `ring-annulus-test-gaps-done.md` case 3. Serves
**daily-drive**: a GTK/libadwaita window rounder than `corner_radius` shows a
strip of desktop background in its corner, beside the ring.

## The gap

PR #882 moved the CSD backdrop into the ring's two strip buffers
(`paint_backdrop_corners`). A strip is only `thickness + radius_outer` rows
tall (`plan_strips`, `strip_target`), but the backdrop square is
`max(24 * scale, radius_inner)` a side, clamped to the window. Whenever the
square is taller than the strip, the backdrop stops at the strip's last row.
The rows below it, inside the client's own transparent corner, are neither
backdrop nor ring (the side bars sit outside the window), so the desktop
shows through as a background sliver down the window's side.

Measured on pixman and the GPU tier (`SCOOT_TEST_RENDERER=cpu|gpu`, scale 1,
client radius 24, `corner_radius = 8`, `focus_ring_width = 2`): drawn rect
`(12, 12)`, outer rect `(10, 10)`, strip height 12 rows, backdrop square 24.
Pixel `(12, 22)`, the first row past the strip, reads the background
`[25, 20, 20]` where the ring `[250, 166, 107]` belongs, and so do the rows
below it down to about `y = 31`, where the client's curve ends. `corner_radius = 12`
with `focus_ring_width = 4` (strip 20 rows) fails the same way. It happens
whenever `thickness + radius_outer` is under `24 * scale`.

Before #882 the backdrop was four separate solid squares, each the full
`max(radius, 24)`, so this did not happen.

## Resolution

`plan_strips` takes `backdrop`; for a CSD window the strip target is
`max(thickness + radius_outer, thickness + side)`. SSD windows keep the
minimal strips. Pinned by the crescent cases in
`compositor/rounded/tests/annulus.rs` and `only_a_backdrop_window_grows_its_strips_to_hold_the_square`.

## Not in this ticket

A different backdrop mechanism (separate elements again).
