---
title: "GLES tier: advertise the renderer's real dma-buf formats and modifiers (tiled, multi-plane), not LINEAR-only"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# GLES tier: full dma-buf format/modifier advertisement

Filed 2026-09-23 (coordinator, after PR #228). Serves **daily-drive** on
real GPUs, and is the precondition for zero-copy video.

## What is wrong

`compositor/dmabuf.rs`'s tranche is `DMABUF_CANDIDATES` — two fourccs
(`Xrgb8888`, `Argb8888`), single-plane, always advertised as `LINEAR`, on
*every* renderer. The renderer only narrows which of those survive. That was
the right fix for the pixman tier (it `mmap`s linear dma-bufs; see the module
doc and `resolved/dmabuf-advertised-but-never-imported-done.md`), and it is
the only shape pixman can honour. Under `--renderer gles` it throws away
what the GPU can actually import:

- **Tiled/compressed modifiers.** A Mesa client on a real GPU told "LINEAR
  only" renders into linear buffers, which is markedly slower to render into
  on most GPUs (and on some, falls back to a copy). Every GPU client on the
  GLES tier pays this today.
- **Multi-plane YUV (`NV12`, `P010`, ...).** Hardware video decoders produce
  these; a player that cannot hand them over as a dma-buf converts on the
  CPU or GPU first. Zero-copy video (decoder → compositor → scanout) needs
  them accepted.

## What to do

Under GLES, derive the advertised set from the renderer's real import set
(`ImportDma::dmabuf_formats()` / `EGLDisplay::dmabuf_texture_formats` at the
pinned rev), fourcc *and* modifier, including multi-plane formats the
renderer imports — the way Smithay's anvil does. Keep pixman exactly as it
is (LINEAR-only, two fourccs, byte-identical advertisement). The module
doc's "promise with teeth" still governs: every advertised
`{fourcc, modifier}` must really import, because a refused `create_immed`
kills the client. That is why this must come from the renderer's own
answer, never a hand list — and the `Modifier::Invalid` subtlety the module
doc records (`imports_linear`) must be carried over, not lost.

Then check every consumer that assumes linear/single-plane: `dmabuf.rs`'s
import path and refusal reasons, screenshots/screencopy reading a
dma-buf-backed surface (GLES renders it, fine; any pixman-side path?),
`schedule_cache_drain`, the scanout exporter (a tiled buffer is fine for
`AddFB2` with modifiers; a multi-plane YUV one goes to the primary only if
the plane lists it — `ANY` does not skip the plane's own format list), and
the primary-direct eligibility (a YUV buffer is opaque; confirm).

## Evidence

The dev VM's EGL device (llvmpipe) imports few modifiers; say what it
advertises before/after. A GL client (`weston-simple-egl`, `glxgears`-style
`es2gears_wayland`, `mpv --vo=gpu`) must keep working under GLES with the new
feedback, and pixman's advertisement must be byte-identical. If a multi-
plane format is advertised on the VM, a real NV12 import must be shown
working (e.g. `mpv --vo=dmabuf-wayland` if it runs, or a probe client).
Real-GPU behaviour (tiled modifiers on Asahi) is an `Asahi.md` runbook step,
not a claim.
