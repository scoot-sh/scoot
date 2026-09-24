---
title: "GLES: resize the render target in place instead of rebuilding the EGL context"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# GLES: resize the render target in place

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**
(`--nested --renderer gles` stutters on every distinct size a drag-resize
passes through).

`docs/tty.md` ("A resize is expensive under `gles`"): every resize rebuilds
the render target, and under `gles` that is a whole new EGL context and
shader set — **16.6 ms** per resize on the dev VM (llvmpipe, 800x800, 8
windows) against **37 µs** for pixman. It names the fix itself: resize the
GLES target in place.

## What to do

Keep the `GlesRenderer`/EGL context across a resize and reallocate only the
offscreen target (and whatever read-back staging depends on its size).
Trace every consumer of the target's size first — damage tracker, read-back
buffers, screenshot/screencopy paths, `render_node` for dmabuf feedback
(which must not change across a resize) — so nothing keeps a stale size.
Pixel-readback suites must stay byte-identical under both renderers.

## Evidence

Before/after resize cost under the same conditions as the 16.6 ms figure
(`CARGO_PROFILE_RELEASE_LTO=thin` on the dev VM — fat LTO OOMs there), plus a
`--nested` drag-resize run. Update `docs/tty.md`'s bullet with the new
number.
