---
title: "Under --renderer gles, every capture of a screen that is not redrawing keeps a whole frame of memory"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# GLES captures on a static screen grow memory by one frame each

Found 2026-09-24 by the scoot/niri A/B (`docs/benchmarks.md`), on the dev VM
(llvmpipe), release build of `fe41921`, `scoot --nested --renderer gles` at
1600x1000 inside cage. pixman is not affected.

## What happens

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

## Why (read from the pinned Smithay rev; not yet proven by a fix)

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

## Why it is high priority

Screenshots of a still screen are the computer-use loop: an agent polls
`scootctl screenshot` while waiting for something to change. Memory grows by
one frame per capture: 8.3 MB at 1080p and 33 MB at 4K. At one capture a
second on a static 1080p screen, that is about 30 GB an hour, and the next
stop is the OOM killer, which takes every client's unsaved state with it.
The trigger is opt-in (`--renderer gles`; pixman is the default). The
`--tty` GPU tiers that capture through the same `read_back` should be
checked too.

## Likely fix, to be measured

After a read-back on the GLES renderer, drain Smithay's cleanup channel by
calling `cleanup_texture_cache()` (public on the `Renderer` trait at the
pinned rev). Alternatively, reuse one mapping per output.

Acceptance is about growth, not release, since RSS may never come back
down: under `--renderer gles` on the dev VM, the 120-shot static sequence
above grows by at most about one frame in total, for both `scootctl
screenshot` and `grim`. There should also be a test that counts live PBOs
or mappings across repeated captures of an undamaged frame, which measures
the queue itself rather than inferring it from RSS.
