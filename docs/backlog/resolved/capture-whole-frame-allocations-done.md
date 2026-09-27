---
title: "Captures allocate a whole frame's worth of memory per capture (read-back and IPC copy) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Captures pooled their whole-frame allocations — RESOLVED

RESOLVED 2026-09-27 (PR #292). Serves agent-driven computer use (screenshot
latency and CPU when an agent polls) and daily-drive (screen recording).

All evidence below was taken on the dev VM (llvmpipe, 4 vCPU), release
builds, `--headless` 1600x1000, and is under
`~/evidence/capture-whole-frame-pools/` there. That directory's `README`
lists every file and binary (sha256-pinned).

## Verdict

Both halves of the ticket's fix landed, plus the GLES analysis the ticket
asked for:

1. **One read-back target per output.** On pixman the persistent
   framebuffer image *is* the read-back target: `Backend::capture` and the
   new `Backend::capture_into` read its own bits directly
   (`render/pixman.rs::framebuffer_bits`) instead of compositing into a
   fresh `pixman::Image` per call. The per-call `Image` is gone on every
   pixman capture path (IPC, `ext-image-copy-capture-v1`, tests).
2. **The worker's `Vec` comes home.** `ShotJob` carries the pool's buffer
   to the worker; `ShotDone` carries it back with the encoded reply, and
   `finish_shot` hands it to the output's pool (`recycle_returned`, also
   used by the two `try_send` failure arms via `recycle_unsent`).
3. **GLES: pooled `Vec`, residual GL-side buffer — measured, not
   half-built.** A pooled GLES read-back avoiding `copy_framebuffer`'s
   per-call pixel-pack buffer is not reachable through the pinned
   Smithay rev's public API: re-binding the target's FBO for a raw
   `glReadPixels` needs `GlesTargetInternal::make_current`, which is
   private, and a `GlesMapping`'s PBO cannot be re-read into without it
   (verified in source at `gles/mod.rs:1641-1680`,
   `ExportMem::copy_framebuffer` at `:1370`). Doing it would mean a new
   Smithay fork commit for a low-priority item, so GLES keeps
   `copy_framebuffer` (with the existing post-capture drain) and pools
   the event-loop `Vec` around it. Measured residual: page faults still
   ~0/shot (llvmpipe's `BufferData` does not fault per call), latency and
   CPU improve with pixman (table below); what remains is one GPU-side
   allocation plus the per-bind FBO per capture, both freed by the drain
   the leak fix added (pinned by the `capture_release` suites, which
   pass unchanged).

## Measured before/after (40 shots/condition, 5 warmup discarded)

| condition                 | base minflt/shot | head minflt/shot | base med ms | head med ms | base jiff/shot | head jiff/shot |
|---------------------------|------------------|------------------|-------------|-------------|----------------|----------------|
| pixman fresh-nocursor     | 3093             | 0.1              | 17.7        | 13.9        | 1.12           | 0.60           |
| pixman fresh-cursor       | 3093             | 0.1              | 18.6        | 13.9        | 1.12           | 0.60           |
| pixman interleaved cursor | 3093             | 0.1              | 17.0        | 14.0        | 1.00           | 0.57           |
| pixman interleaved nocurs | 3093             | 0.1              | 16.4        | 13.7        | shared         | shared         |
| gles fresh-nocursor       | 3093             | 0.1              | 17.3        | 13.6        | 1.10           | 0.60           |
| gles fresh-cursor         | 3093             | 0.1              | 17.0        | 14.8        | 1.00           | 0.60           |
| gles interleaved cursor   | 3093             | 0.1              | 16.3        | 14.4        | 0.93           | 0.60           |
| gles interleaved nocursor | 3093             | 0.1              | 15.3        | 13.2        | shared         | shared         |

Page faults per screenshot go from ~3093 (two fresh full-frame
allocations: the read-back copy and the encode input) to ~0 in every
mode on both renderers. Wall latency drops ~3-4 ms (~20-25%),
compositor CPU from ~1.0 to ~0.6 jiffies/shot (~40%).

## Lifetime trace (the 5b bug class)

A reused read-back target raced by concurrent captures would hand one
capture another's pixels. Traced at every site:

- `take_capture_buf` removes the buffer from the pool; only its taker
  ever recycles it. The event loop is single-threaded, and the worker
  owns its `Vec` exclusively from `try_send` to `ShotDone` -- a move,
  never shared state. Two outstanding takes are two allocations (pinned
  by `take_and_recycle_keep_empty_buffers_and_no_sharing`).
- `finish_shot` recycles before any reply logic, so no early return
  leaks the buffer; a removed output drops it (`recycle_returned` files
  only under a live `OutputId`, pinned by
  `a_buffer_for_a_removed_output_is_dropped_not_kept`).
- A job no worker will answer (`Full`, `Disconnected` at `try_send`)
  returns its buffer via `recycle_unsent`. A buffer whose worker died
  with it is lost with it -- the pool refills on demand, and the orphan
  reap is unchanged.
- Kept buffers are always empty (`recycle_capture_buf` clears;
  `debug_assert` in `take_capture_buf`): the pool keeps capacity, never
  pixels.
- The global 4-capture bound (`MAX_IN_FLIGHT_SHOTS`) and the
  per-connection ordering refusal are unchanged and still hold --
  `CAPTURE_BUFS_KEPT` is pinned equal to the bound, and a
  connection-tests suite plus the full IPC suites pass unmodified.
- No per-frame heap added anywhere: `take`/`recycle` run only on
  capture dispatch/completion; the render loop, `cursor_changed` and
  the frame element paths are untouched.

## Tests

- `render/tests/capture_pool.rs` (new): `capture_into` byte-equals the
  borrowed `capture` on both renderers; second fill reuses the
  allocation (pointer + capacity stable); take/recycle emptiness and
  no-sharing; pool bound; bound-equality with `MAX_IN_FLIGHT_SHOTS`;
  GLES in-place resize reuses the buffer at the new size.
- `ipc::connection::tests::a_finished_encode_returns_its_buffer_to_the_output_pool`:
  two real IPC screenshots at 1600x1000 share one allocation by pointer
  identity.
- `screenshot::tests::a_buffer_for_a_removed_output_is_dropped_not_kept`.
- Full cheap set green on the dev VM: `nextest -p scoot -p scoot-core
  -p scoot-ipc -p scootctl` 2081 passed; `SCOOT_TEST_RENDERER=gles`
  1785 passed on `-p scoot` (the 7 `dmabuf::tests` import failures are
  the documented udmabuf-provenance known failures, failing at `Failed
  vs Created` before any read-back runs); `clippy -p scoot --all-targets -D
  warnings` clean; `fmt --check` clean; `smoke-test.sh` rc=0 with 22
  ok under pixman and under `RENDERER=gles`.
- README: no user-facing surface changed (no config, keybinding, CLI
  flag or IPC shape) -- a latency/CPU improvement only -- so no README
  update, stated explicitly.
- Noted, not touched (out of scope): `clippy --features gpu-scanout
  --all-targets` flags `scanout_frame_elements` (8 args) identically on
  `main`; one `scootbg` socket-backlog test fails with EMFILE on this
  VM (another agent's lane, untouched).

## The original ticket

Filed 2026-09-23 from the review of PR #231 (capture cursor parity).
Serves **agent-driven computer use** (screenshot latency and CPU when an
agent polls) and daily-drive (screen recording).

### What is wrong

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

### Why it matters

The review of PR #231 measured IPC screenshot wall latency moving with page
faults and with process history, not with the capture's own work. The
+3.5 ms median first attributed to the cursor region flips sign when
cursor and no-cursor captures are interleaved in one process (default
9.1 ms against no-cursor 10.2 ms; fresh processes 12.6 against 10.4). The
region itself costs 0.4-0.9 ms. So capture latency on this path is
dominated by allocation behaviour this code does not control yet.

### What to do

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
