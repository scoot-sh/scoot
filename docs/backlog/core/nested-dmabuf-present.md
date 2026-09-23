---
title: "--nested --renderer gles: present to the host by dmabuf instead of read-back + wl_shm"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# `--nested` GPU presentation by dmabuf

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
- Host resize interplay with [GLES resize in place](./gles-resize-in-place.md).

## Evidence

Before/after CPU per frame under the same damage load on a host with a
GPU; on the dev VM (llvmpipe) only functional correctness and fallback are
provable — say so.
