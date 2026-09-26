---
title: "Transitions between wallpapers (milestone 2)"
status: "open"
area: "scootbg"
priority: "medium"
blocked: null
---

# Transitions between wallpapers (milestone 2)

Fade, wipe (with an angle), grow/outer from a point, and `none`, with a
duration and an easing curve.

- CPU only, into `wl_shm`: per frame, blend old and new scaled buffers
  into a third. Pace by frame callbacks, and use `wp_presentation` where
  offered to drop frames rather than fall behind.
- Damage only what changed each frame (a wipe's moving band), so the
  compositor redraws the least.
- A new request mid-transition starts from what is on screen now; no
  queueing of stale transitions.
- A per-frame time budget, measured on the dev VM under pixman at 4K. If a
  transition cannot hold it, it degrades (fewer steps), never stutters the
  compositor.
- Once finished, free the extra buffers and return to zero idle cost.

Benchmarks before and after are required: this is the first hot path in
scootbg.
