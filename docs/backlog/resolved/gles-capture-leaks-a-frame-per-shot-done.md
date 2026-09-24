---
title: "Under --renderer gles, every capture of a screen that is not redrawing kept a whole frame of memory — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GLES captures on a static screen grew memory by one frame each — RESOLVED

RESOLVED 2026-09-24 (PR #238). Found by the scoot/niri A/B (PR #237, whose
open ticket `docs/backlog/core/gles-capture-leaks-a-frame-per-shot.md` this
closes). All evidence below was taken on the dev VM (llvmpipe, 4 vCPU,
kernel 6.18.50), and all of it is under `~/evidence/gles-capture-leak/`
there. That directory's `README` lists every file and binary.

## Root cause, as proven

The ticket's hypothesis was right, and it was the whole story. Read from
the pinned fork (`scoot-sh/smithay` `43f50eb`) and then measured.

- `ExportMem::copy_framebuffer` on `GlesRenderer` makes a pixel-pack
  buffer the size of the region (`gles/mod.rs:1370`). For a screenshot
  that region is the whole frame. Binding the target makes a framebuffer
  object as well: `Bind<GlesRenderbuffer>` at `:1641`, and the texture arm
  of `Bind<Dmabuf>` (`bind_texture`, `:754`) on the scanout tier.
- Dropping either handle does not delete the GL object. `GlesMapping`'s
  `Drop` (`texture.rs:224`) and `GlesTargetInternal`'s `Drop` (`:264`) send
  it down a cleanup channel. Only `GlesRenderer::cleanup()` (`:820`)
  deletes what is in the channel. That runs from a frame's `finish`
  (`:2564`), `unbind` (`:805`), `cleanup_texture_cache` (`:2346`) and
  `invalidate_caches` (`:2355`).
- A capture of a screen nothing redraws reads the persistent target and
  reaches none of those. Each capture's pixel-pack buffer therefore stayed
  allocated until the next frame was drawn, and on a static screen no
  frame is drawn.

**Pinned by a test, not inferred from RSS.** `LiveGlObjects`
(`render.rs`, test-only) counts live GL buffer and framebuffer names with
`glIsBuffer`/`glIsFramebuffer`. An object still in the queue answers true,
so the count measures the queue itself.

- `a_dropped_read_back_buffer_stays_allocated_until_the_queue_is_drained`
  pins the premise on the pinned rev: a held mapping is +1 buffer, the
  bind is +1 framebuffer, both are still live after their handles drop,
  and both are gone after a drain. It also shows the probe can see a
  queued object at all.
- Against the unfixed code (`01-failfirst-tests-unfixed.txt`), 12 captures
  of an unchanged frame went from `buffers: 3, framebuffers: 2` to
  `buffers: 15, framebuffers: 14`. That is +1 of each per capture through
  `Backend::capture` and through the IPC path. Through fresh
  `ext-image-copy-capture-v1` sessions it was 8 captures, +8 of each.

**What the memory was.** On llvmpipe the pixel-pack buffer's storage is
ordinary process heap, so the queue showed up as RSS: 6,250 KiB per capture
at 1600x1000, which is exactly one ARGB frame. On the nested dma-buf tier
the inner output was 782x976 (the host tiled it at half width). There the
growth was 2,980–2,984 KiB per shot, and 782×976×4 = 2,981.5 KiB, again one
frame (`dmabuf-before-ipc-per-shot.txt`). On a real GPU this would be
driver or GPU memory rather than process heap. That has not been measured
here (see "Not verified" below).

## The release pattern the ticket could not explain

The ticket saw three things it could not explain. Rendering frames did not
bring RSS down. Captures stopped growing once frames had been drawn.
Mapping a window dropped RSS. Two findings explain all three.

1. **A drawn frame does free the queue.** `finish` drains it. This is
   measured directly with glibc's dynamic mmap threshold turned off
   (`GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072`). With a fixed
   threshold, every frame-sized allocation is its own `mmap` and is
   `munmap`ped on free, so RSS tracks what is live. The before binary
   (`run-before-nested-ipc-fixed.txt`):

   ```
   empty, no clients              rss_kb=127956
   after 120 shots                rss_kb=878372
   one foot mapped (static)       rss_kb=135368    <- one frame drawn: all 120 freed
   +20 shots                      rss_kb=260416    <- static again: +6,250 KiB/shot
   animating 10 s (frames drawn)  rss_kb=144764    <- freed again
   +20 shots static again         rss_kb=263624
   ```

2. **With the default allocator settings, freed memory mostly stays in the
   process.** glibc starts with a 128 KiB mmap threshold and raises it to
   the size of the first large `mmap`ped chunk freed. After that,
   frame-sized allocations come from the heap, and `free` only gives
   memory back when the top of the heap is free past the trim threshold.
   So the drained buffers were freed, and the high-water mark stayed. How
   much came back depended on what else sat above them on the heap. In
   this run a mapped window gave back 891,568 → 251,892 KiB
   (`run-before-nested-ipc-dyn.txt`). In the ticket's run, 42 s of
   animation gave back almost nothing. "Captures stopped growing once
   frames had been drawn" was the same thing seen from the other side: a
   frame between captures drained the queue, so each new buffer reused
   memory the allocator already held.

So the ticket's acceptance criterion (growth, not release) was the right
one. The fix stops the growth, and with it the high-water mark never forms.

## The fix

`gles::release_captured` (`render/gles.rs`) calls
`Renderer::cleanup_texture_cache`, which runs `GlesRenderer::cleanup()`
(the drain every frame's `finish` runs). It is called once the capture has
returned and its mapping and framebuffer have dropped, whether the capture
succeeded or not:

- in `Backend::capture`'s `Gles` arm (offscreen GLES: `--headless`,
  `--nested` read-back and the `--nested` dma-buf tier), which is the funnel
  for IPC screenshots and `ext-image-copy-capture-v1`;
- in its `Scanout` arm (`--tty` GPU tier), after the refusal check, so a
  refused capture does no GL work and has nothing to drain;
- in the GLES capture-cursor region render
  (`PatchTarget<GlesRenderer>::render_into`, `render/capture_cursor.rs`),
  on every path out, `?` included. The region render's own `finish`
  drains the queue *before* its read-back, so without this the region's
  buffer and framebuffer object waited for the next frame.
  `02-mutation-patch-drain.txt` shows the IPC-with-pointer test failing
  with this one drain removed.

Nothing else is drained, and nothing changes for pixman. pixman's
`copy_framebuffer` makes an image that is freed on drop and never queued.
The Smithay fork is not modified.

**Deliberately left alone: the per-frame presenter read-back**
(`draw_frame`, which under GLES means `--nested` presenting by read-back
into `wl_shm`). The next frame's `finish` drains it, so at most one is ever
outstanding, and it exists only because a frame was drawn. Draining there
would add a `make_current` to every frame of the render loop for no
growth it prevents. A comment at the call site says so.

**Why not reuse one pixel-pack buffer.** Smithay's `ExportMem` at the
pinned rev has no way to read into an existing mapping.
`copy_framebuffer` always generates a new buffer. Draining straight away
gets the same effect through the allocator: the freed frame-sized chunk is
the one the next capture's allocation gets.

## Evidence

All numbers are RSS in KiB, taken from `/proc/PID/smaps_rollup`, 1600x1000
unless marked. Each run takes 120 captures of a static screen 50 ms apart,
then waits 10 s, maps a `foot`, takes 20 more captures, animates a client
for 10 s, stops it, and takes 20 more captures. "Before" is `fe41921`:
`~/evidence/niri-ab/bin/scoot` (default features) or
`bin/scoot-before-gpu-fe41921` (`gpu-scanout`). "After" is `cb39f10`,
built both ways. Default glibc settings unless marked.

| Tier | Capture | Before: empty → 120 shots | After: empty → 120 shots |
|---|---|---|---|
| `--nested` read-back (in cage) | `scootctl screenshot --no-cursor` | 128,864 → 891,568 | 128,856 → 129,176 |
| `--nested` read-back | `grim` | 128,904 → 879,260 | 128,828 → 129,168 |
| `--nested` read-back | `scootctl screenshot` (pointer drawn in) | 128,856 → 148,220 | 128,820 → 129,400 |
| `--headless` | `scootctl screenshot --no-cursor` | 109,356 → 872,096 | 109,400 → 116,000 |
| `--headless` | `grim` | 109,392 → 860,164 | 109,400 → 116,084 |
| `--tty` GPU scanout tier | `scootctl screenshot --no-cursor` | 106,408 → 869,172 | 106,464 → 113,092 |
| `--tty` GPU scanout tier | `grim` | 106,556 → 857,196 | 106,536 → 113,184 |
| `--nested` dma-buf tier (782x976) | `scootctl screenshot --no-cursor` | 107,568 → 471,588 | 107,600 → 116,820 |
| `--nested`, `--renderer pixman` | `scootctl screenshot --no-cursor` | (not re-run: code unchanged) | 35,428 → 35,744 |
| `--nested`, `--renderer pixman` | `grim` | (not re-run) | 35,428 → 35,908 |

Before, every GLES row grew 125,000–125,040 KiB per 20 shots (6,250 KiB
per shot), except the dma-buf tier at its own frame size. After, every row
is flat from the 20th shot to the 120th. In the headless, tty and dma-buf
rows, the one step in the first 20 shots is 6.6–9.2 MB, about one frame:
the first capture's buffer, which the allocator keeps for the next one to
reuse. This meets the ticket's acceptance criterion ("grows by at most
about one frame in total"). The later sequence stays flat too: after a
mapped window, while animating, and after the animation stops (the
`run-after-*` files). Before, the "+20 shots static again" step grew by
another 20–40 MB and went on growing. With the fixed threshold, the after
binary held 121,828 KiB from the 20th shot to the 120th
(`run-after-nested-ipc-fixed.txt`).

**The pointer.** Before the fix, an IPC screenshot *with* the pointer
(`scootctl screenshot`'s default) did not grow on the nested tier (the
table's third row). A frame there never holds the pointer, so every such
capture re-rendered the pointer's region, and that render's `finish`
drained the previous captures' objects. The leak needed `--no-cursor`, or
an `ext-image-copy-capture-v1` client that does not paint cursors
(`grim`'s default), or a tier whose frame already holds the pointer. That
last case was not measured here.

**Capture latency.** `bench.sh` measured `scootctl screenshot --no-cursor`
end to end: client start, capture, PNG encode on scoot's worker, file
written. 100 shots at 5 Hz, `--nested` 1600x1000 with one `foot` mapped,
after 5 warm-up shots (`bench-5hz.txt`, per-shot µs in `bench-raw-*.us`):

| | p50 ms | p95 ms | RSS after, KiB |
|---|---|---|---|
| before, gles (run 1 / run 2) | 8.83 / 8.77 | 9.62 / 9.57 | 804,916 / 804,940 |
| after, gles (run 1 / run 2) | 7.85 / 7.89 | 8.79 / 8.70 | 148,672 / 148,668 |
| before, pixman (control) | 10.80 | 11.68 | 42,704 |
| after, pixman (control) | 10.67 | 11.65 | 42,704 |

The fixed GLES capture is about 1 ms *faster* at both p50 and p95. That is
consistent with the next capture reusing a chunk whose pages are already
faulted in, where before each capture faulted in 6.25 MB of fresh heap.
That explanation is a reading of the numbers and has not been measured
separately. pixman's capture path is unchanged, and its two runs agree to
within 0.13 ms.

**Tests** (`render/tests/capture_release.rs`,
`screencopy/tests.rs::captures_of_a_static_screen_on_gles_leave_no_gl_objects_behind`).
They name `RendererKind::Gles`, so they run in the default suite and need
EGL, like the other GLES tests in `render/tests.rs`. Each one asserts two
things: no growth from the first capture to the last, and nothing left
queued, meaning an explicit drain afterwards deletes nothing. The second
is stronger. A path that left one frame queued after every capture would
pass "no growth". Covered: `Backend::capture` directly, the IPC path with
and without the pointer, and fresh ext-capture sessions with and without
painted cursors.

**A metric that did not work.** `probe.sh` also printed `frame_maps`, the
count of anonymous VMAs of one frame's size. It stayed at 0 or 1 even
when RSS showed 120 live buffers under the fixed threshold. The kernel
merges adjacent anonymous mappings into one VMA, so VMAs do not count
allocations. The column is kept in the raw files, and nothing here relies
on it.

## Not verified

- **Real GPU hardware.** On a real GPU the buffers are driver or GPU
  memory, not process heap, so neither the leak nor the fix is visible in
  RSS there. The mechanism is the same code path. The GL-object count
  that the tests assert is what would show it. Real hardware is
  `Asahi.md`'s territory, and this change adds no new test there.
- **A tier whose frame already holds the pointer**, with `scootctl
  screenshot`'s default pointer-on capture. By the code, that capture
  renders no region and so drained nothing before the fix. It now drains
  like every other capture.
