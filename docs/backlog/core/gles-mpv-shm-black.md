---
title: "GLES tier: mpv --vo=gpu (wl_shm via Mesa swrast, subsurfaces + viewport) intermittently composites black or striped"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# GLES tier: mpv `--vo=gpu` intermittently shows black or striped

Found 2026-09-23 while gathering client evidence for
[gles-dmabuf-full-formats](../resolved/gles-dmabuf-full-formats-done.md)
(PR #229). **Present before that change**, and intermittent rather than
solid: it is not a format problem (mpv never sends a dma-buf on the dev VM;
it takes Mesa's `wl_shm` swrast path).

## What was seen

Dev VM, `--headless --renderer gles` (llvmpipe), mpv 0.41.0
`--no-config --vo=gpu --gpu-context=wayland --loop` on a 320x240 `testsrc2`
clip, one `scootctl screenshot` about 6 s in. The client plays (its log
counts frames, it commits a new `wl_shm` buffer every ~100-200 ms) and stays
connected. The same client under `--headless` (pixman) shows the test
pattern correctly. `es2gears_wayland` and `gtk4-demo --run=video_player`,
also on `wl_shm` through swrast, drew correctly under GLES in every run.

Outcomes per run, one screenshot each:

| who | build | runs |
| --- | --- | --- |
| implementer | before (`a3b883e`) | black |
| implementer | after, headless gles / `--tty` gles | black / black |
| review | before (`a3b883e`) | black, black, striped, striped |
| review | after (PR head) | correct, correct, striped, striped |

"Striped" is sparse dotted rows over the video area. Correct, black and
striped all occurring on the same build points to a race (between mpv's
buffer commits, the subsurface/viewport state they come with, and the frame
the capture reads), not to a deterministic mis-render -- and it means
one screenshot per run proves nothing either way.

What mpv does that the clients that always draw correctly do not (from
`WAYLAND_DEBUG=1`): four surfaces with a `wp_viewport` each, two
`wl_subsurface`s (`#7 <- #6 <- #5`), the video on the innermost subsurface
with `set_destination(782, 976)` and `set_opaque_region`, and
`damage_buffer(0, 0, i32::MAX, i32::MAX)` per commit.

Evidence (dev VM): `~/evidence/gdf/client-gles-mpv-gpu.*`,
`client-tty-gles-mpv-gpu.*`, `client-BEFORE-gles-mpv-gpu.*` (the
pre-change binary), `client-pixman-mpv-gpu.*`; the script is
`~/evidence/gdf/gl-client.sh`.

## To do

1. **Measure it as a rate, not a picture.** Loop N runs (N >= 20) per
   configuration, and for each take several screenshots a few hundred ms
   apart. Score each with a pixel metric over the window's content rect,
   e.g. the fraction of pixels that match `testsrc2`'s bar colours within a
   tolerance (correct: most of the video rect; striped: a sparse fraction;
   black: ~0). Report the distribution per build and per renderer; pixman is
   the control and should score correct every time.
2. **Bisect with that loop**, one mpv behaviour at a time in a scratch
   client: viewport-scaled `wl_shm` on a subsurface; `i32::MAX` damage on a
   subsurface; a synchronised vs desynchronised subsurface; the black
   single-colour backdrop surfaces mpv stacks under the video.
3. Fix, and pin with a pixel-readback test that runs under
   `SCOOT_TEST_RENDERER=gles` -- repeated, since a single frame cannot show
   a race.
