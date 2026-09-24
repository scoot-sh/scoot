---
title: "--nested --renderer gles: present to the host by dmabuf instead of read-back + wl_shm — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--nested` GPU presentation by dma-buf — RESOLVED

RESOLVED 2026-09-24 (PR #235, branch `nested-dmabuf-present`). In a
`gpu-scanout` build, a `--nested --renderer gles` session whose host
composites on the renderer's own DRM device copies each frame on the GPU
into a host dma-buf instead of reading it back into `wl_shm`. Read-back
stays for everything else, chosen once at startup and logged. The design
record is `crates/scoot/src/compositor/nested/gpu.rs`'s module doc; the
answers to the ticket's questions are below.

## Design answers

- **Build shape: behind `gpu-scanout`, no new feature.** Allocating a host
  buffer needs GBM (link-time libgbm) or an EGL Wayland-platform window
  surface (`libwayland-egl`, link-time unless dlopened, plus a second EGL
  display). `gpu-scanout` already is "the feature that links libgbm", so
  the axis is one feature per link-time cost; a sibling would double the
  flake's `scoot-gpu` packaging for the same library. The default build
  still links no GPU stack (the `ldd` pair in the PR). The name predates the
  second user; `Cargo.toml` and `docs/nix.md` say so.
- **Device matching.** The host's v4 default feedback is read once, on a
  private event queue with one blocking round trip, and parsed as
  adversarial input (`pread` at offset 0, whole-entry sizes only, a cap at
  what `u16` indices can name, every index and `dev_t` length checked; any
  malformation means read-back). The host's `main_device` must be the same
  DRM device as the renderer's own node, compared through `DrmNode`'s
  render node (the protocol forbids comparing `dev_t`s, and the dev VM's
  hosts name the render node where another may name the primary); only
  tranches targeting that device count. A different device (a split-GPU
  host on the other GPU) is read-back -- the renderer's device is never
  moved (PR #229's pin). GBM is opened on the renderer's render node, then
  the same device's primary node: Mesa's `kms_swrast` (the dev VM) cannot
  allocate on a render node (`CREATE_DUMB: Permission denied`) and can on
  the primary.
- **Formats/modifiers.** `Argb8888` (what the read-back presents, so a
  translucent background looks the same), else `Xrgb8888`; explicit
  modifiers both listed by the host for that device and in the renderer's
  `dmabuf_render_formats`, scanout-flagged tranches first; `Invalid`
  (implicit) only when nothing explicit is common *and* the host lists it,
  never mixed with explicit (PR #229). GBM's answer is checked against
  what the host accepts, because Smithay's allocator silently retries
  without modifiers when a list naming `Linear` fails. A startup probe
  allocates, binds and blits one small buffer, so a driver that cannot
  (no GLES 3 blit, a layout it imports but cannot render into) is a
  read-back decision, not a failed first frame.
- **The copy.** The frame is still composited into the persistent GLES
  render target, then `glBlitFramebuffer`'d into the host buffer, and the
  sync point waited on before the commit. Rendering into the host buffers
  directly would save the blit but move what every capture reads (and the
  damage tracker's history) onto buffers the host holds; the blit keeps
  captures correct by construction.
- **Swapchain.** At most three host buffers per size, shared through
  `zwp_linux_buffer_params_v1.create` (a refusal is an event; `create_immed`
  would make it a fatal error on scoot's own host connection). A chain
  starts with one buffer and grows only when a frame finds every existing
  one held and none still being created. A frame with no usable buffer is
  owed and handed over as drawn by the next `release`/`created` -- which
  is also every resize's first frame, drawn before the host has created the
  new chain (without that, every resize drew its first frame twice).
  `created`/`failed` carry the chain's generation; a stale `created` has its
  buffer destroyed.
- **Interplay.**
  - *Resize* (PR #232): a size over the GLES limit is now refused before
    any host buffer (or `wl_shm` pool) is allocated for it, so no client is
    told a mode that never renders; an allocation failing at a new size is
    the old "staying at the previous size", never a fallback.
  - *Screenshots/screencopy*: read the render target, unchanged
    (`copying_a_frame_out_leaves_what_captures_read_untouched`; live, host
    and nested screenshots byte-identical).
  - *Presentation-time and frame callbacks*: **the ticket's premise was
    wrong** -- `--nested` has never used host frame callbacks
    (`nested_dispatch.rs` registers no `wl_callback`); frames are paced by
    scoot's own timer, unchanged. Presentation feedback is stamped when the
    commit is flushed, by the render tail or, for a handed-over frame, by
    the hand-over itself.
  - *Host refusing a buffer mid-session*: read-back for the rest of the
    session, one WARN; the `wl_shm` pool is built at the current size, and
    if even that fails the next frame retries it.
  - *VT/host focus loss*: `--nested` has no VT; a host that stops
    releasing buffers while hidden leaves frames owed (never more than
    three buffers), handed over on the next release.

## Measured (dev VM, llvmpipe; release, LTO off, codegen-units 16)

`scripts/nested-dmabuf-bench.sh` at `9b79daa` (the final code), host an
outer `scoot --headless --renderer gles` (cage cannot host this on the VM:
under `gles2` it has no output, under pixman no linux-dmabuf), 1024x768,
both builds from that commit, the same host binary, alternated twice:

| per | read-back (default build) | dma-buf (`gpu-scanout`) |
|---|---|---|
| frame (30 Hz counter, 20 s), nested | 7.70 / 7.77 ms | 7.73 / 7.67 ms |
| frame, host | 14.82 / 14.85 ms | 15.05 / 14.82 ms |
| applied size (121 sizes), nested | 14.13 / 14.30 ms | 15.12 / 15.12 ms |
| applied size (1001 sizes), nested | 12.70 ms | 13.66 ms |

So on llvmpipe the dma-buf path is CPU-neutral per frame and ~6-8% dearer
per resize (about 0.9 ms a size): drawing the frame in software is nearly
all of the cost, llvmpipe's blit is itself a CPU copy, and each size now
allocates two host buffers (not measured apart; kernel-zeroed dumb buffers
and their first mapping are the likely share). What it saves on a real GPU
is `Asahi.md` Test 8, not a claim. Across 1001 resizes fds stayed flat on
both processes (nested 42, host 30) and RSS moved as much as the read-back
run's; 2003 frames were handed over as drawn (two per size: the first, and
the second, which waited for the chain to grow to two buffers); host and
nested screenshots byte-identical in every run.

Evidence (commands, SHAs, raw paths) is in the PR description.

## Original ticket

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**:
`--nested` is the project's first, low-risk step for trying scoot on real
hardware (`CLAUDE.md`), and it is the one place `--renderer gles` buys
nothing on a GPU box — every frame is composited on the GPU, read back to
main memory and presented to the host as `wl_shm`.

**This argues against a recorded position, deliberately.** `docs/tty.md`
and the resolved planes ticket call headless/nested read-back "design, not
TODO" (headless has no CRTC; nested presents bytes to its host). That is
right for pixman and for headless. It is not forced for nested + gles: the
host compositor almost certainly advertises `zwp_linux_dmabuf_v1`, and a
buffer allocated on the host's `main_device` can be rendered into and
attached directly.

## What to do (design first)

- Answer the build-shape question before writing code: allocating host
  buffers needs GBM (link-time libgbm, which is why `gpu-scanout` is a
  feature) or an EGL Wayland-platform window surface (`libwayland-egl`, also
  linked unless dlopened). The default build must keep linking no GPU stack
  (`ldd` check in the smoke/CI). Probably: behind `gpu-scanout` (or a sibling
  feature), with read-back as the fallback when the host has no dmabuf, no
  matching device, or refuses the buffer.
- Host dmabuf feedback gives the device and formats; render device and host
  device must match (or the path is refused, with read-back).
- A small swapchain (2–3 host buffers), released on `wl_buffer.release`,
  with damage passed through to `wl_surface.damage_buffer`.
- Host resize interplay with [GLES resize in place](../resolved/gles-resize-in-place-done.md).

## Evidence

Before/after CPU per frame under the same damage load on a host with a
GPU; on the dev VM (llvmpipe) only functional correctness and fallback are
provable — say so.
