---
title: "A layer surface's exclusive zone can take a whole output"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# A layer surface's exclusive zone can take a whole output

Filed 2026-09-29. Serves **daily-drive**: one client's exclusive zone must not
leave the user with no usable area.

## The gap

Found reviewing the scootbar skeleton (PR #323). On an 800x600 or 1920x1080
headless scoot, `scootbar daemon --height 1024 --margin 1024` (both within its
flag bounds) makes `scootctl outputs` report a usable area of
`{x:0, y:0, w:1920, h:0}`. A spawned `foot` was tiled to 942x1 at (12,12). scoot
did not crash. The reported `y:0` for a zone that starts at 2048 also looks
wrong. `LayerMap::arrange` in the pinned Smithay fork
(`src/desktop/wayland/layer.rs`, ~370-389) sums zone and margin with saturating
arithmetic and does not bound the result by the output.

Any layer client can do this with a large exclusive zone, so it is not specific
to scootbar.

## What to do

Decide whether scoot clamps the total reserved zone per output (leaving a
minimum usable area), and what usable area and `y` it then reports. Pin the
edge cases: zones on opposing edges that sum past the output, and a zone
removed while windows are tiled into the remainder.

## Not in this ticket

scootbar's own flag bounds (its CLI, its entry).
