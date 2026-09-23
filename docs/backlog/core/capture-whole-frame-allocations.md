---
title: "Captures allocate a whole frame's worth of memory per capture (read-back and IPC copy)"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Captures allocate a whole frame's worth of memory per capture

Filed 2026-09-23 from the review of PR #231 (capture cursor parity).
Serves **agent-driven computer use** (screenshot latency and CPU when an
agent polls) and daily-drive (screen recording).

## What is wrong

Every capture allocates buffers the size of the whole output, fresh, and
drops them after. Both IPC `screenshot` and `ext-image-copy-capture-v1` pay
for this, with or without the pointer:

- **The read-back** (`render::read_back`, `ExportMem::copy_framebuffer` at
  the pinned Smithay rev):
  - pixman allocates a new `pixman::Image` of the region per call.
  - GLES creates a new pixel-pack buffer per call, and binding the
    scanout tier's recorded dma-buf goes through Smithay's per-bind FBO.
- **IPC `screenshot`** then copies those pixels into an owned `Vec`
  (`screenshot.rs`'s `read_back`, `<[u8]>::to_vec`), about 6.4 MiB at
  1600x1000. That copy has to move to the encode worker, so pooling it
  means handing buffers back from the worker.

At these sizes the allocator hands each buffer back to the kernel on free,
so each capture pays page faults over megabytes of fresh memory.

## Why it matters

The review of PR #231 measured IPC screenshot wall latency moving with page
faults and with process history, not with the capture's own work. The
+3.5 ms median first attributed to the cursor region flips sign when
cursor and no-cursor captures are interleaved in one process (default
9.1 ms against no-cursor 10.2 ms; fresh processes 12.6 against 10.4). The
region itself costs 0.4-0.9 ms. So capture latency on this path is
dominated by allocation behaviour this code does not control yet.

## What to do

- Keep one read-back target per output, as the cursor region's target now
  is (`render::capture_cursor::PatchPool`). Under GLES that needs a
  read-back that does not go through `copy_framebuffer`'s per-call
  pixel-pack buffer.
- For IPC, recycle the worker's `Vec` back to the event loop along with
  the encoded reply.
- Measure page faults (`/proc/<pid>/stat` minflt), screenshot latency and
  CPU, before and after, on pixman and on the GPU tier, with cursor and
  no-cursor captures interleaved in one process as well as in fresh ones.

The frame path's own per-frame element lists (the arrangement, each
source's element `Vec`), which the capture region's gather shares, are a
related but separate question: they are small, and the render loop pays
them on every frame.
