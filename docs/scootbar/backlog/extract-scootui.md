---
title: "Extract `scootui`: the shared drawing and Wayland scaffolding"
status: "open"
area: "scootbar"
priority: "low"
blocked: "a second consumer: popups, the launcher or scootnotify"
milestone: "M7"
---

# Extract `scootui`

Filed 2026-09-29. Serves **daily-drive** (one look across the shell) and the
lightness bar (no duplicated code in three binaries).

Write the drawing and scaffolding as modules inside `scootbar` first. Extract
when a second consumer needs them, so the seam comes from real use rather
than guesswork.

## Likely shape

- **`scootui`**: pure drawing with no Wayland: a canvas over `&mut [u32]`,
  rects and rounded rects, the glyph cache and text layout, color tokens, and
  the popup widgets. Snapshot-testable and fuzzable.
- **Wayland scaffolding**: per-output layer-surface lifecycle across hotplug,
  the shm pool, fractional scale. scootbg already has most of it
  (`crates/scootbg/src/outputs.rs`); decide whether it joins the extraction.
- **Line-framed JSON socket framing**, once a third daemon needs it (scootbg,
  scootbar, scootnotify).

## Guard rails

scootbg has a release gate. Touching it means re-running its benchmark and
showing no regression beyond the margin, or leaving it on its own copy and
saying so. Feature-gate the parts a consumer does not use so the smallest
binary does not grow.

## Done when

Two binaries build on it, both benchmarks are unchanged within the margin, and
no consumer pays for a part it does not use.
