---
title: "Buffers keep their fds past their own wl_buffer object, uncounted; multi-plane dma-bufs count as one"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Buffer fds retained past their object

Filed 2026-09-24 while fixing
[client-held fd bound](../resolved/client-held-fd-bound-done.md). It serves
**daily-drive**, for the same reason that one did: fds a client can make scoot
hold that no per-client cap counts are fds that shed *other* clients under
fd pressure, and the pressure guard cannot pick the holder.

## What is wrong

1. **A committed buffer outlives its object.** The renderer's per-surface
   state holds a handle to the committed `wl_buffer`, and a wayland-server
   handle keeps the object's user data alive after the object is
   destroyed. For shm that user data holds the pool, which owns the fd and
   the mapping. So `create_pool`, `create_buffer`, attach and commit on a
   fresh surface, destroy the buffer and the pool, repeat: the buffer and
   pool counts are back at zero every iteration, and each surface keeps one
   fd and one mapping (up to the 512 MiB pool cap each). Measured in the
   harness (pixman, `618b5dc` + the client-held-fd-bound branch, scratch
   test kept at `~/evidence/cfb/scratch_surface_held.rs` on the dev VM):
   200 surfaces, **200 fds held, 0 buffers and 0 pools counted**. That was
   shm only. A dma-buf buffer should behave the same (its `Dmabuf` owns the
   plane fds, and the handle keeps it alive the same way), on every tier;
   reasoned from the same code path, not measured.
2. **The buffer count weighs every buffer as one fd.** A dma-buf `wl_buffer`
   holds one fd per plane, up to four. pixman refuses multi-plane imports,
   so on the default tier it is one; under a GLES renderer (`--renderer
   gles`, the `--tty` GPU scanout tier) multi-plane YUV and modifier layouts
   import, so 512 buffers can be ~2048 fds from one client. Reasoned from
   the import path, not measured.

A third, related number: on the GPU scanout tier one connection at every
cap at once already sums past the pressure reserve on its own (`fd_pressure.rs`
states it: ~865 counted + 43 baseline against a 896 line).

## Direction

Count fds, not objects, for what outlives its object. The
`drm_syncobj/retained.rs` ledger already does that for timelines: it
records each client fd by number when it arrives, forgets it when the number
comes back or a sweep finds it closed, and decides refusals only on a fresh
sweep. Extending it to pool and plane fds (every fd a client hands scoot
that scoot keeps) gives an exact per-client total, which could replace
the per-kind arithmetic above with one per-client fd budget and grace.
Weigh the per-`add`/`create_pool` cost of recording (one hash insert; the
sweep stays off the hot path), and whether a shm buffer's shared pool fd
should count once per pool or per buffer (once: it is one fd).
