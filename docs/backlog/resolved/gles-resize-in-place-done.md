---
title: "GLES: resize the render target in place instead of rebuilding the EGL context — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GLES: resize the render target in place — RESOLVED

RESOLVED 2026-09-23 (PR #PRNUM, branch `gles-resize-in-place`).

## What landed

- **A GLES resize reallocates the renderbuffer alone**
  (`GlesBackend::resize`, reached through `Backend::resize_in_place` from
  `State::resize_output`), on the renderer the backend already has: the EGL
  context, its shaders, every imported client texture and the EGL device
  stay. All or nothing: the new target is allocated *and* bound (the
  framebuffer-completeness check is the only thing that catches a size over
  `GL_MAX_RENDERBUFFER_SIZE`; `glRenderbufferStorage` only sets a GL error
  flag, which Smithay clears before each of its own checks) before the old
  one is released, and the old one is freed in the same call rather than
  queued to the next frame's cleanup.
- **Everything size-bound follows it**: the recorded size (what
  `refresh_capture_constraints` re-advertises to capture clients), a fresh
  damage tracker (the same `from_output` one `Backend::new` builds; headless
  and nested pass age 0, so a full redraw either way) and an empty
  `CursorInFrame` (no frame has drawn into the new target). What does not
  depend on the output's size and so stays: the read-back's pixel-pack
  buffer (made per call by `copy_framebuffer` at the pinned rev), PR #231's
  capture-cursor pools (`PatchPool`, `patch_pixels`; keyed on the region
  and scale), `render_node`/`main_device` (the same display), the device
  pin, presentation feedback and frame callbacks (not touched by a resize).
- **A refused size falls back to a whole new backend, pinned to the
  session's device** (read off the backend still in `backends`), and only if
  that fails too is the resize refused as before (old mode put back, the
  refused one deleted from the mode list). Known and accepted: for a size
  over `GL_MAX_RENDERBUFFER_SIZE` the fallback is certain to fail too, and
  pays a context build to find out. Reachable from `--nested` only through a
  host configure between the driver's limit and `MAX_OUTPUT_DIMENSION`
  (65535); zero never reaches `resize_output` (`usable_size`).
- **pixman is untouched** (it still rebuilds, in microseconds), and the
  `--tty` scanout tier still resizes through `DrmCompositor`.
  `add_output` still builds a new pinned backend.

## Measured (dev VM, llvmpipe)

`headless::bench::resize_cost` (800x800, 8 windows, best of 5 runs of 40),
release with `CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`
on both trees -- not the thin LTO the ticket asked for, because a thin-LTO
build of a second tree exhausted the VM's memory and wedged it; the 16.6 ms
in the original ticket was that profile's number and does not compare.
Ranges are across all runs. The frame after an in-place resize varies by
run in a pattern that repeated in all three invocations (runs 1-2 ~0.28 ms,
run 3 ~1.5 ms, runs 4-5 ~0.64 ms); not investigated -- every run is still
~3x-17x under the rebuild it replaced:

| | before (`7ff7265`) | after |
|---|---|---|
| resize, gles | 3.95 ms | 15.7 µs |
| resize + the frame after, gles | 4.35 ms (4.35–4.43) | 0.26 ms (0.26–1.56, median 0.64) |
| resize, pixman | 11.3 µs | 11.3 µs |
| resize + frame, pixman | 52.6 µs | 51.9 µs |

A `--nested --renderer gles` drag under cage (the nested smoke's host;
`scripts/nested-drag-bench.sh`, 150 distinct sizes, a `foot` mapped):
17.0 / 17.3 ms of compositor CPU per size before, 14.7 / 15.1 after, every
size applied in place and none by a new renderer; pixman on the same drag
4.5 ms. What is left is llvmpipe drawing and reading back a whole frame at
the new size. RSS flat across 1000 oscillating resizes. Screenshots after
the drag byte-identical before and after.

Evidence (commands, SHAs, raw paths) is in the PR description.

## Original ticket

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
