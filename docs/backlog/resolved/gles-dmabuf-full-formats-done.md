---
title: "GLES tier: advertise the renderer's real dma-buf formats and modifiers (tiled, multi-plane), not LINEAR-only — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GLES tier: full dma-buf format/modifier advertisement — RESOLVED

RESOLVED 2026-09-23 (PR #229, branch `gles-dmabuf-full-formats`).

## What landed

- **The GLES tranche is the driver's import set** (`dmabuf.rs::driver_tranche`,
  fed by `Backend::dmabuf_import_set`, i.e. `ImportDma::dmabuf_formats` =
  the EGL display's `dmabuf_texture_formats`), on both GLES tiers: every
  fourcc at every explicit modifier the driver named, external-only entries
  included (they import as `GL_TEXTURE_EXTERNAL_OES`, which every
  `GlesRenderer` can sample), `Xrgb8888`/`Argb8888` first, then the driver's
  order, one group per fourcc. **pixman is byte-identical** (same function
  as before, renamed `cpu_mapped_tranche`; the wire table diffed empty, below).
- **`Modifier::Invalid` is resolved, not passed through.** Smithay inserts
  `{fourcc, Invalid}` into the texture *and* render sets unconditionally,
  so an implicit YUV buffer is bound `GL_TEXTURE_2D` against the driver's
  external-only answer. Measured on llvmpipe: `NV12`/`P010`/`YU12`/`YUYV`
  filled red and sent at `Invalid` import fine and draw **0** red pixels of
  1024; at `LINEAR` all 1024. So implicit is never advertised next to an
  explicit answer (wlroots' `init_dmabuf_formats` draws the same line, as
  read in the archived `swaywm/wlroots` mirror; anvil advertises the whole
  set, which is not the evidence here). A fourcc with no explicit answer
  survives only as a candidate at `LINEAR`, which is exactly
  `imports_linear`'s old case.
- **`imports_linear`'s doc overstated its soundness, corrected.** It said no
  modifier attribute reaches the `EGLImage`; with the modifiers extension
  present the `LINEAR` attribute *is* attached (`egl/display.rs:817-824`),
  so on a driver that names tiled layouts and not `LINEAR` the old rule
  advertised `LINEAR` on `Invalid` evidence alone — a latent `create_immed`
  refusal. The GLES rule no longer does (it offers the named layouts);
  pixman is unaffected.
- **Downstream audit.** `schedule_cache_drain` and the per-commit sync are
  renderer-agnostic / pixman-only respectively; capture reads the composited
  framebuffer, never a client buffer; Smithay's `has_alpha` knows no YUV
  fourcc so YUV is opaque to both renderer and occlusion (consistent;
  alpha-carrying YUV like `AYUV` composites opaque — noted in the module
  doc); plane indices are bounded at 4 in Smithay's dispatch.
- **Two things review found that did need code** (review of PR #229):
  - *A GLES rebuild could change device.* `GlesBackend::new` re-ran "first
    device that builds wins" on every resize and added output, while the
    feedback (now carrying one driver's tiled modifiers) is never re-sent: a
    transient failure on a two-GPU machine could move a backend to a device
    that refuses what clients were promised — a `create_immed` kill.
    Rebuilds are now pinned to the first build's `EGLDeviceEXT`
    (`render::gles::GlesDevice`, `State::gles_device`) and fail rather than
    migrate; `add_output`'s failure path now also takes back the
    `wl_output` global and `Space` mapping it had already made.
  - *The primary-direct safety argument had the mechanism wrong.* A
    single-plane `LINEAR` buffer at offset 0 is GBM-imported without
    modifiers and `AddFB2`'d without one on **every** device, so that path
    rests on the kernel's implicit layout for an imported linear buffer
    being linear (true wherever measured). And a GBM that loses a tiled
    modifier would give **scrambled tiles, not a fallback**: the fb becomes
    `{fourcc, Invalid}`, which every plane lists (`drm/mod.rs:288-297`).
    The exporter is now wrapped (`tty/layout_exporter.rs`): a client
    framebuffer that did not keep the client's explicit non-`LINEAR`
    modifier is dropped and the element composites. `Asahi.md` Test 6 still
    asks whether any real GBM does it.
- **Known trade-off, not a regression of anything measured:** with tiled
  layouts on offer, a fullscreen GL client on real hardware may allocate a
  layout the display cannot scan out and composite instead of going
  primary-direct. Steering it is [the scanout tranche](./gpu-scanout-candidates-done.md).
  Primary-direct has only ever been seen on the dev VM, whose table is
  still `LINEAR`-only.

Evidence (commands, SHAs, raw paths) is in the PR description.

## Original ticket

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
