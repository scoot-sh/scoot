---
title: "dma-buf capture buffers for ext-image-copy-capture-v1"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# dma-buf capture buffers for `ext-image-copy-capture-v1`

`screencopy::constraints()` returns `BufferConstraints { dma: None, .. }`, so
a capture session offers `wl_shm` buffers only. Filed 2026-09-19, out of
milestone 6 stage 4, which was expected to cover it and should not have: the
two look adjacent and are not the same work.

## Why it is not part of "renderer-derived dmabuf formats"

Stage 4 made the `zwp_linux_dmabuf_v1` tranche derive from what the active
renderer can **import** — a client's buffer coming *in*. Capture is the other
direction, where scoot does the writing, and the capability that governs it is
a different one:

- **It needs a write path that does not exist.** Filling a client's dma-buf
  means `Bind`ing it as a render target and blitting the frame into it.
  Today `deliver` writes through `with_buffer_contents_mut`, i.e. `wl_shm`
  only. That is a new per-capture failure surface on every renderer.
- **The format set is the other one.** What may be offered here is the
  renderer's `dmabuf_render_formats` (what it can render *into*), not the
  `dmabuf_texture_formats` stage 4 derives. On the scanout tier the two are
  already known to differ — `tty/scanout.rs` keeps the render set for
  `DrmCompositor::new` and says in as many words that it never reaches the
  client advertisement.
- **It needs a `DrmNode` the primary target does not have.**
  `DmabufConstraints` requires one. On the GPU-less containers scoot is built
  for there is no DRM node at all, so the honest answer there stays `None`
  whatever else changes — this can only ever be a conditional offer.
- **It trips a documented landmine on the default renderer.**
  `dmabuf.rs`'s `schedule_cache_drain` records that binding a dma-buf render
  target *under pixman* makes every `wl_buffer` destruction in the session
  evict the bound-target cache (upstream's second `retain` drops entries whose
  `dmabuf` is `None`) and force a re-`mmap` on the next frame. The GPU scanout
  tier escapes that because `GlesRenderer::cleanup` retains differently; a
  pixman capture path would not. Narrowing that drain has to land in the same
  change.

## What would justify doing it

A measured client need, which is the same bar `screencopy.rs`'s module doc has
always set for this. Nothing observed so far asks for it: `grim` captures into
`wl_shm`, and quickshell's `ScreencopyView` needs the dmabuf *global* to exist
(which it does — that is what gates its readiness) rather than dmabuf capture
buffers. A screen-*sharing* pipeline that wants zero-copy into an encoder is
the plausible one, and is worth measuring against the shm path before
assuming it is faster: on a CPU renderer the frame is already in main memory.
