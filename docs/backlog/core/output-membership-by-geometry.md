---
title: "Three readers still answer \"which output is this window on\" from its bounding box, not its placement"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Output membership read from geometry

Filed 2026-09-23 while fixing
[windows bleeding across outputs](../resolved/windows-bleed-across-outputs-done.md),
which made drawing and hit-testing follow the output a window is *placed*
on. Three other sites still decide by bounding-box overlap, so a window
whose rect crosses a shared edge (a column scrolled part-way off its output,
a fullscreen window focused away from) counts as being on the neighbour too.
None of them draws or delivers input to the wrong place, which is why they
were left out of that fix; each is a smaller inconsistency:

- **Frame callbacks** (`headless.rs`, the per-output loop after each frame):
  a window overlapping an output's geometry is sent a frame callback with
  that output as the token. An overhanging window's callback is fired by
  whichever of the two outputs renders first, so it is paced (and its
  presentation feedback attributed) by a screen it is not shown on.
- **`wl_surface.enter`/`leave`** (Smithay's `Space::refresh`, called from
  `headless.rs`): the client is told its surface entered the neighbouring
  output. Harmless while every output shares one scale; once per-output
  scale lands (`per-output-scale-mode.md`) a client picks its buffer scale
  from the outputs it has entered, and would pick the neighbour's.
- **`foreign_toplevel_management.rs`'s `output_of_window`**: reads the first
  output whose geometry overlaps the window's bbox, so a handle announced
  (or a `wl_output` bound) while a window overhangs the output before its
  own names the wrong `output_enter`; the next `apply()`'s membership diff
  (which reads the placement) corrects it.

## What to do

Read the placed output (`output_clip::placed_on`, stamped by `apply()` from
the placement) at all three. The frame-callback change is the one with an
observable effect (which output's frame fires an overhanging window's
callback); pin it with a harness test that renders only the neighbouring
output and asserts the callback stays pending. The
`wl_surface.enter` half means not relying on `Space::refresh`'s overlap
bookkeeping for windows -- check what else reads `Space::outputs_for_element`
before replacing it.
