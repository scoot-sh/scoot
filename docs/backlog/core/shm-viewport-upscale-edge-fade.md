---
title: "A tiny wl_shm buffer upscaled by wp_viewporter fades at its edges under pixman"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# A tiny wl_shm buffer upscaled by wp_viewporter fades at its edges under pixman

Found while measuring scootbg's dependencies (2026-09-26, see
[`docs/scootbg/backlog/resolved/dependencies-done.md`](../../scootbg/backlog/resolved/dependencies-done.md)),
not yet reproduced independently of that prototype.

On `scoot --headless` (pixman), a client that commits a 1×1 `XRGB8888`
`wl_shm` buffer and sets a `wp_viewporter` destination of the whole output
gets a gradient instead of a flat colour: for `#c03020` the centre sampled
`srgba(187,46,31,0.976)` and a corner `srgba(48,12,8,0.251)`. That reads as
bilinear sampling reaching past the edge of a one-pixel texture (sampling
transparent outside it) rather than clamping or repeating the edge. An
`XRGB` buffer should also never come out with alpha below 1.

The same colour through `wp_single_pixel_buffer_v1` renders exactly, and
scootbg uses that path on scoot, so scootbg is not affected. Any other
client that upscales a small shm buffer is (a 1×1 solid-colour fallback is
common in wallpaper tools that support compositors without single-pixel
buffers).

To do: reproduce with a minimal client, check the GLES renderer too, and
fix the sampling (edge clamp / pad the source, or treat an opaque format's
alpha as 1) with a pixel test at the output corners.
