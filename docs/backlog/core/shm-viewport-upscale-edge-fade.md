---
title: "Upscaled surfaces get a semi-transparent 1-px edge under pixman (Smithay samples past the texture edge)"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Upscaled surfaces get a semi-transparent 1-px edge under pixman (Smithay samples past the texture edge)

Found while measuring scootbg's dependencies (2026-09-26, see
[`docs/scootbg/backlog/resolved/dependencies-done.md`](../../scootbg/backlog/resolved/dependencies-done.md)),
then reproduced independently and widened in the review of PR #266.

## What happens

Under the pixman renderer (the default), any surface whose buffer is
scaled *up* to its on-screen size fades toward transparent black at its
edges instead of keeping its edge pixels:

- **A 1×1 `XRGB8888` `wl_shm` buffer viewported to the whole output**
  (`#c03020`, `scoot --headless`): centre `srgba(187,46,31,0.976)`, corner
  `srgba(48,12,8,0.251)`, left-middle `srgba(95,23,15,0.494)`. The whole
  surface is a gradient.
- **Every buffer-scale-1 surface on an output with scale > 1**, fractional
  scales included: a full logical-size buffer with no viewport, on
  `[output] scale = 2.0`, has a 1-px edge at `srgba(108,27,18,0.561)`
  around an interior of `srgba(192,48,32,1)`. That is every client that
  does not do HiDPI itself, and likely XWayland windows, on any scaled
  output. On XRGB scanout the edge shows as a dark line.

Both the colour channels and alpha fade (premultiplied toward black), so an
`XRGB` surface comes out partly transparent too.

`wp_single_pixel_buffer_v1` renders exactly, which is why scootbg (it uses
that path on scoot) is unaffected. The GLES renderer sets `CLAMP_TO_EDGE`
(`gles/mod.rs:947` in the pinned Smithay), so this is almost certainly
pixman-only.

## Cause

In the pinned Smithay fork (`~/.cargo/git/checkouts/smithay-*/5b57532`),
`src/backend/renderer/pixman/mod.rs:591-597` pairs `Filter::Bilinear` with
`src_image.set_repeat(Repeat::None)`. Bilinear taps at the edge then read
outside the image, where pixman returns `(0,0,0,0)`, and an opaque source
is composited with `Operation::Src`, which writes the faded result straight
into the framebuffer. scoot's own code sets no filter or repeat on this
path.

## Fix

`Repeat::Pad` on the source image, pixman's equivalent of GL's
clamp-to-edge. (Forcing alpha to 1 for opaque formats would *not* fix it:
the colour channels still fade, giving an opaque dark vignette instead.)

The code is Smithay's, so the fix is a commit on the scoot-sh/smithay fork,
listed in [`docs/forks.md`](../../forks.md) in the same PR, per `CLAUDE.md`'s
fork policy. Test with pixel samples at the corners and edge midpoints for
both cases above (1×1 viewported, and scale-1 on a scale-2 output), plus a
check that downscaled surfaces are unchanged.

## Priority

Medium: a visual defect only (no crash, disconnect or lost work, so not a
harm under `CLAUDE.md`'s rules), but it touches most windows on any HiDPI
output under the default renderer, which is a daily-use problem.
