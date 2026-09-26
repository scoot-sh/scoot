---
title: "A wallpaper per workspace (milestone 3)"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# A wallpaper per workspace (milestone 3)

Through `ext-workspace-v1`, the standard protocol scoot already offers, so
this works on any compositor that has it, not only scoot.

- Map workspace (by name or index) to an image per output; switch with a
  transition when the active workspace changes.
- Decide how much to preload: switching must be instant, but holding every
  workspace's 4K buffer in memory is not free. Measure.
