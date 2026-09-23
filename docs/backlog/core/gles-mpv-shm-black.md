---
title: "GLES tier: mpv --vo=gpu (wl_shm via Mesa swrast, subsurfaces + viewport) composites black"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# GLES tier: mpv `--vo=gpu` shows a black window

Found 2026-09-23 while gathering client evidence for
[gles-dmabuf-full-formats](../resolved/gles-dmabuf-full-formats-done.md).
**Not caused by that change**: the same binary pair was run before and after
it, and both are black.

## What was seen

Dev VM, `--headless --renderer gles` (llvmpipe), mpv 0.41.0
`--no-config --vo=gpu --gpu-context=wayland --loop` on a 320x240 `testsrc2`
clip. The client plays (its log counts frames, it commits a new `wl_shm`
buffer every ~100-200 ms) and stays connected, but the window is solid
black in `scootctl screenshot`. The same client under `--headless`
(pixman) shows the test pattern correctly. `es2gears_wayland` and
`gtk4-demo --run=video_player`, also on `wl_shm` through Mesa's swrast
path, draw correctly under GLES.

What mpv does that those do not (from `WAYLAND_DEBUG=1`): four surfaces
with a `wp_viewport` each, two `wl_subsurface`s (`#7 <- #6 <- #5`), the
video on the innermost subsurface with `set_destination(782, 976)` and
`set_opaque_region`, and `damage_buffer(0, 0, i32::MAX, i32::MAX)` per
commit. One of those is the difference between the renderers; which one
is not established.

Evidence (VM paths): `~/evidence/gdf/client-gles-mpv-gpu.*`,
`client-BEFORE-gles-mpv-gpu.*` (the pre-change binary),
`client-pixman-mpv-gpu.*`.

## To do

Bisect the four candidates with a scratch client (viewport-scaled shm on a
subsurface; `i32::MAX` damage on a subsurface; stacking of the black
single-colour surfaces mpv uses as a backdrop), fix, and pin with a
pixel-readback test that runs under `SCOOT_TEST_RENDERER=gles`.
