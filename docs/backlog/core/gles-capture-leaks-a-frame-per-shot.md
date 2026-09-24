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
memory by **6.25 MB**, which is one 1600x1000 ARGB frame (6.4 MB). The growth
is linear and nothing gives it back while the screen stays still. Both
capture paths do it: `scootctl screenshot` and `grim`
(ext-image-copy-capture). Raw, from
`~/evidence/niri-ab/probe-gles-shot-growth.txt` on the dev VM:

```
empty, no clients          rss_kb=122408 pss_anon_kb=39928
after 10 shots             rss_kb=197728 pss_anon_kb=115056
after 50 shots             rss_kb=447732 pss_anon_kb=365060
after 100 shots            rss_kb=760240 pss_anon_kb=677568
after 120 shots            rss_kb=885240 pss_anon_kb=802568
30 s idle after            rss_kb=885240 pss_anon_kb=802568
one foot mapped            rss_kb=383184 pss_anon_kb=290388
```

The next rendered frame releases most of it (the `one foot mapped` line).
Once frames are flowing, captures stop growing. The animate-scene lines in
`~/evidence/niri-ab/probe-memory-growth.txt` show this: 20 captures during an
animation grew memory by 200 kB. The same file shows pixman and niri staying
bounded under the same sequence.

## Why (read from the pinned Smithay rev, not yet proven by a fix)

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
frame is drawn.

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
pinned rev). Alternatively, reuse one mapping per output. Acceptance: the
120-shot sequence above stays flat on the dev VM under `--renderer gles`,
for both `scootctl screenshot` and `grim`. There should also be a test that
counts live PBOs or mappings across repeated captures of an undamaged
frame.
