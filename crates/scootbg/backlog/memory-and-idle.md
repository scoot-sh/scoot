---
title: "Buffers, memory and zero idle cost"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Buffers, memory and zero idle cost

The resource budget is the feature. Targets to measure and publish in
`crates/scootbg/README.md` once the static path exists:

- **Idle:** no wakeups at all with a static wallpaper (no timers, no frame
  callbacks requested). Verify with `perf stat` / wakeup counts over a
  minute, the way `docs/benchmarks.md` measures the compositor.
- **Buffers:** one `XRGB8888` buffer per output in a `memfd` pool, opaque
  region set, reused in place on a same-size redraw. A 4K output is ~33 MB;
  two 4K outputs showing one image share nothing but the source decode.
- **RSS:** record resident memory for 1× 1080p, 1× 4K and 2× 4K, after the
  source has been dropped.
- **Startup:** time from `scootbg daemon` to the first committed buffer
  with a restored 4K JPEG.

No `release` event handling tricks until measured: a static wallpaper
commits once, so double buffering only matters for transitions.
