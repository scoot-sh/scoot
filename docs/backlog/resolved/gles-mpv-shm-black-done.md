---
title: "GLES tier: mpv --vo=gpu (wl_shm via Mesa swrast, subsurfaces + viewport) intermittently composites black or striped — CLOSED WITHOUT CODE (client-side: mpv/Mesa presents mid-render shm, evidence below)"
status: "resolved"
area: "core"
priority: "medium"
blocked: null
---

# GLES tier: mpv `--vo=gpu` intermittently shows black or striped

Found 2026-09-23 while gathering client evidence for
[gles-dmabuf-full-formats](./gles-dmabuf-full-formats-done.md)
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

## Resolution (closed without code, 2026-09-26)

**Verdict: not a compositor bug.** The black/striped outcomes are states of
mpv 0.41.0's own `wl_shm` present buffers under Mesa llvmpipe swrast —
pre-present (slow first frame) and mid-render (llvmpipe still rasterizing
when the buffer is committed) — and scoot displays the committed bytes
faithfully on both renderers. No scoot change exists that would make those
bytes otherwise; per the ticket ("do not invent a fix"), none is made, and
step 2/3's fix-and-pin do not apply. What follows is the evidence, recorded
so anyone can re-derive it with `scripts/mpv-shm-rate/`.

All runs below: dev VM, `--headless`, mpv 0.41.0
(`/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv`)
`--no-config --vo=gpu --gpu-context=wayland` on
`~/evidence/gdf/testsrc-nv12.mkv` (320x240 `testsrc2`, 20 s), scoot built
from `d3b75a5` (post-#254; `scoot 0.1.0`, binary sha `00cadf0ed1bbb69e`).
Metric: ImageMagick mean/std over the window content rect (window rect from
`windows`, inset 8 px). Calibration on viewed shots: empty background
mean 0.250865 std 0.00134; solid testsrc2 frames std 0.24–0.33 depending on
the animation frame (a big grey cross lowers it — the metric conflates
content with fidelity, so every class below was confirmed by eye, and
fidelity itself is anchored on the pool-byte proof, not on thresholds).

Rate tables (N=20 runs, 4 screenshots ~300 ms apart per run,
`scripts/mpv-shm-rate/run.sh` + `score.sh`; every run unanimous 4/4):

| renderer | black runs | dotted/mid runs | solid runs |
| --- | --- | --- | --- |
| gles | 9 | 8 | 3 |
| pixman (control) | 3 | 13 | 4 |

Pixman is not the clean control the ticket expected — it varies run to
run too, with the same dotted structure. That already points away from the
GLES texture path: pixman imports shm with a memcpy, so identical dots on
both renderers means the dots are in the committed bytes.

Pool-byte proof (`pool-bytes.sh`, `catch-dots.sh`): with mpv `--pause`d,
`/proc/<mpv>/fd` shows its 2–3 `mesa-shared` pools (3052928 B =
782x976x4 each); copying one straight out of `/proc` and decoding it as
`BGRA` renders the same dotted-row bands the screenshot shows, at the same
rows. A black-run pool dump has all three pools byte-identical
(md5 `8b08df60fc554665e943739e45f663f7`) with mostly zeros and sparse
`0x00010101` words — pre-render contents, never committed (the screenshot
shows background grey, not buffer black). One GLES run caught the
unambiguous shape: a razor-straight diagonal frontier, dotted below-left,
black above-right — llvmpipe's tile wavefront mid-render.

Controls that isolate the compositor: `mpv --vo=wlshm --pause` under
pixman is solid and frozen (std 0.333502 twice); `es2gears_wayland`
(`gl-control.sh`, 6 runs x 2 shots x pixman/gles) is rock-stable
(mean 0.3002x, std 0.1406x, only animation jitter) — GL shm clients that
present finished buffers display correctly on both renderers today.

Commit traffic (`timeline-debug.sh`, WAYLAND_DEBUG, gles, 33 s): 2523
commits + 2522 attaches, all on the video surface, releases 2520,
`wl_callback.done` 4004 — the compositor keeps up (releases ≈ attaches,
callbacks fire from every rendered frame's tail) while the screenshots
stay byte-identical. Stale-texture is impossible on that path anyway: a
new attach clears the surface's texture cache and both renderers then do
a full upload (GLES takes the `upload_full || damage.is_empty()` arm on
empty damage). Frozen screen + flowing releases + firing callbacks means
the client re-committed unchanged bytes. Related: with no vsync headless,
callbacks fire immediately and mpv commits at ~76 Hz with `Dropped: 112`
by mid-clip — it presents whatever llvmpipe has finished, including
mid-render buffers.

Ticket premises corrected along the way: the video attaches on the root
toplevel surface, not "the innermost subsurface" (the two subsurfaces
never attach or commit); the viewport is 1:1 (`set_destination(782, 976)`
for a 782x976 buffer), not scaled; `damage_buffer(0,0,i32::MAX,i32::MAX)`
is real but irrelevant once the pool bytes are read — damage only selects
upload rects, and full-rect damage uploads full bytes. The "never
dma-buf" premise holds for presented buffers: `params.add` 0,
`create_immed` 0 in every WAYLAND_DEBUG run (the `udmabuf`/`lp_dma_buf`
fds in mpv's table are llvmpipe-internal and never cross the protocol).

Harness kept in `scripts/mpv-shm-rate/`: `run.sh` (rate loop),
`score.sh` (ImageMagick scorer), `timeline-debug.sh` (screenshots plus
commit/attach/release/callback counts), `paused-vo.sh` (vo isolation),
`pool-bytes.sh` + `catch-dots.sh` (pool-vs-screenshot proof),
`gl-control.sh` (es2gears stability). No `SCOOT_TEST_RENDERER` pin test:
there is no compositor behavior to pin.

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
