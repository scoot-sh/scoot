---
title: "Skeleton: one bar layer surface per output, across hotplug"
status: "open"
area: "scootbar"
priority: "high"
blocked: "baselines-and-spikes (dependency choices)"
---

# Skeleton: one bar layer surface per output

Filed 2026-09-29. Serves **daily-drive**.

The crate, a `scootbar daemon` that connects, tracks every output as it comes
and goes (as scootbg does, `crates/scootbg/src/outputs.rs`), and gives each
one a `top` layer surface anchored to a chosen edge with an exclusive zone
and namespace `scootbar`. It draws a solid bar and nothing else.

## What to do

- Crate `crates/scootbar`, Linux only (a stub elsewhere, like scootbg), the
  same release profile and `unsafe` discipline; shm buffers via the existing
  `scootbg-mem` pieces if they fit, otherwise the smallest local equivalent.
- Draw at real device pixels including fractional scales (`wp_viewporter` +
  `wp_fractional_scale`), a pooled double buffer, damage on change only.
- Bar height and edge from config defaults; the exclusive zone must equal the
  bar so windows never jump when the first buffer lands (scoot applies the zone
  from the buffer-less initial commit, `layer-surface-bufferless-exclusive-zone-done.md`).
- Reuse scootbg's per-output lifecycle rather than reinventing it; note what
  would move into a shared crate later ([extract-scootui](extract-scootui.md)),
  but do not refactor scootbg here.

## Tests

Headless scoot: the surface exists per output, the zone shifts a window, an
output added and removed at runtime adds and removes a bar, and a screenshot
over IPC has the bar color where expected (scootbg's integration pattern).
Edge cases: zero outputs, an output removed mid-frame, a scale change.

## Done when

`scootbar daemon` on headless scoot shows a solid bar on each output, and
idle wakeups are zero.
