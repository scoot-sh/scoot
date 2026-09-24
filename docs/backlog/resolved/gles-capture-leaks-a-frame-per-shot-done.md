---
title: "Under --renderer gles, every capture of a screen that is not redrawing kept a whole frame of memory — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GLES captures on a static screen grew memory by one frame each — RESOLVED

RESOLVED 2026-09-24 (PR #238). Found by the scoot/niri A/B (PR #237). The
open ticket it filed is folded in verbatim at the end of this file, under
"The original ticket". All evidence below was taken on the dev VM (llvmpipe, 4 vCPU,
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

**The pointer.** A default IPC screenshot, which draws the pointer in,
was not safe before the fix. It only happened to stay flat in the one case
measured here. On `--nested` (the table's third row) a frame never holds
the pointer, so every such capture re-rendered the pointer's region. That
render's `finish` drained the previous captures' objects, and one frame's
worth stayed queued. When a capture renders no pointer region, nothing
drains, and a default screenshot leaked exactly like `--no-cursor`. That
happens when the pointer is hidden or off the output
(`a_hidden_cursor_needs_nothing_unless_the_frame_still_shows_one`), and
when the frame already holds the pointer where it is, as on a GPU tier
that composites the cursor rather than giving it a plane
(`a_frame_that_composited_the_cursor_where_it_is_needs_nothing_when_asked`).
Neither case was measured live here; the dev VM's scanout tier has a
cursor plane. `grim`, and any `ext-image-copy-capture-v1` client that does
not paint cursors, leaked in every case measured.

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

- **Real GPU hardware.** On a real GPU the buffers are driver
  allocations, not process heap. They count in RSS only while mapped into
  the process, so the leak may or may not have shown there. The mechanism
  is the same code path. The GL-object count that the tests assert shows
  it either way. `Asahi.md` Test 9 now says what to check instead: RSS
  should stay flat across its screenshot lines, and so should DRM fdinfo
  memory where the driver reports it. This change adds no new hardware
  test.
- **A default (pointer-on) screenshot where no pointer region is
  rendered**: a hidden pointer, a pointer off the output, or a tier whose
  frame already composites the pointer. By the code, before the fix that
  capture drained nothing and leaked like `--no-cursor`. It now drains
  like every other capture. It was not reproduced live.

## The original ticket (filed by PR #237, verbatim)

Kept for the diagnosis history. Its open questions are answered above.

### GLES captures on a static screen grow memory by one frame each

Found 2026-09-24 by the scoot/niri A/B (`docs/benchmarks.md`), on the dev VM
(llvmpipe), release build of `fe41921`, `scoot --nested --renderer gles` at
1600x1000 inside cage. pixman is not affected.

#### What happens

Each screenshot taken while nothing on screen is redrawing grows scoot's
memory by **6,250 kB** (6.4 MB decimal), which is one 1600x1000 ARGB frame:
6,400,000 bytes, or 6,250 KiB. The two figures are the same size in
different units. The growth
is linear and does not stop. Both capture paths do it: `scootctl
screenshot` and `grim` (ext-image-copy-capture). pixman and niri stayed
bounded under the same sequence
(`~/evidence/niri-ab/probe-memory-growth.txt` on the dev VM).

Raw data from `~/evidence/niri-ab/probe-gles-shot-growth.txt`, an empty
session, captures 50 ms apart:

```
empty, no clients          rss_kb=122408 pss_anon_kb=39928
after 10 shots             rss_kb=197728 pss_anon_kb=115056
after 50 shots             rss_kb=447732 pss_anon_kb=365060
after 100 shots            rss_kb=760240 pss_anon_kb=677568
after 120 shots            rss_kb=885240 pss_anon_kb=802568
30 s idle after            rss_kb=885240 pss_anon_kb=802568
one foot mapped            rss_kb=383184 pss_anon_kb=290388
+10 shots                  rss_kb=383192 pss_anon_kb=290396
```

What happens after the growth is only partly understood. Record it as
observed:

- **Rendering frames did not bring RSS down.** In
  `probe-memory-growth.txt`, 42 s of an animating client (frames drawn
  continuously) left scoot-gles at 467 MB, where 50 captures had put
  it. In the benchmark's main run, the "end" row after the animate scene is
  still 289.8 MB, against 283.6 MB after the screenshot scenes.
- **After frames had been drawn, further captures stopped growing.** That
  was 20 captures during the animation, and 10 captures after the foot in
  the table above mapped.
- **Mapping a new window dropped RSS once, but not always.** In the table
  above, mapping a foot took RSS from 885 MB to 383 MB (`one foot mapped`).
  In `probe-memory-growth.txt`, mapping the animation's foot did the
  opposite, taking RSS from 461.6 MB to 467.9 MB (`after 20 grim shots` to
  `anim +2s`). Neither is explained, and the one drop should not be read as
  a release mechanism.

These fit the mechanism below if RSS keeps its high-water mark
(the allocator retains freed memory for reuse and rarely returns it). On
that reading, the next rendered frame frees the queued buffers, later
captures reuse them, and RSS stays high. That is a hypothesis. RSS does not
count PBOs directly, and nothing here has measured the queue.

#### Why (read from the pinned Smithay rev; not yet proven by a fix)

`render::read_back` calls `copy_framebuffer`, which on `GlesRenderer`
allocates a pixel-pack buffer the size of the region, and then
`map_texture`. Dropping the `GlesMapping` does not delete that PBO. Drop
only sends `CleanupResource::Mapping` down the renderer's cleanup channel
(`src/backend/renderer/gles/mod.rs`, `GlesCleanup::cleanup`, line 327 at
`43f50eb`). The channel is drained only by `GlesRenderer::cleanup()`, and
that runs from `unbind()`, `cleanup_texture_cache()` and
`invalidate_caches()`, which are the render path's calls. A capture of an
undamaged screen reads the persistent target without rendering, so nothing
drains the channel, and each capture's PBO stays allocated until the next
frame is drawn. That explains the growth. It does not by itself explain the
release pattern above.

#### Why it is high priority

Screenshots of a still screen are the computer-use loop: an agent polls
`scootctl screenshot` while waiting for something to change. Memory grows by
one frame per capture: 8.3 MB at 1080p and 33 MB at 4K. At one capture a
second on a static 1080p screen, that is about 30 GB an hour, and the next
stop is the OOM killer, which takes every client's unsaved state with it.
The trigger is opt-in (`--renderer gles`; pixman is the default). The
`--tty` GPU tiers that capture through the same `read_back` should be
checked too.

#### Likely fix, to be measured

After a read-back on the GLES renderer, drain Smithay's cleanup channel by
calling `cleanup_texture_cache()` (public on the `Renderer` trait at the
pinned rev). Alternatively, reuse one mapping per output.

Acceptance is about growth, not release, since RSS may never come back
down: under `--renderer gles` on the dev VM, the 120-shot static sequence
above grows by at most about one frame in total, for both `scootctl
screenshot` and `grim`. There should also be a test that counts live PBOs
or mappings across repeated captures of an undamaged frame, which measures
the queue itself rather than inferring it from RSS.
