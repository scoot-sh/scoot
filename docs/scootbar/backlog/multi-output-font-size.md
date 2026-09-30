---
title: "Per-output font size"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M3"
---

# Per-output font size

Filed 2026-09-30, the remainder of [multi-output](resolved/multi-output-done.md). Serves
**daily-drive**: a 4K panel next to a 1080p one wants a different em, not
only the same em at its own device pixels.

## The gap

`[output."NAME"]` (see [cli.md](../cli.md#outputs)) overrides the bar's
geometry and module lists, but the em (`[bar] font-size`), like every part
of `Style`, is shared: `Content.style` is one value that `Scene::update` and
`render::paint` read for every output. Each bar already draws at its own
output's real device pixels, so the same 14 logical pixels are sharp at any
scale; a different size per output is not expressible.

## What to do

Add `font-size` to the output table, validated as `[bar] font-size` is (1 to
256), and carry the size per output instead of in the shared `Style`: the
scene measures at it, `paint` and the click hit-test read the output's own.
The glyph cache is keyed by size already, so a second size costs its glyphs
only. Pin: a reload that changes an output's size re-measures and redraws
only that output, and `padding` and `spacing` stay shared unless asked.

## Not in this ticket

Per-output colors, padding, spacing, radius and opacity.
