---
title: "Ring annulus test gaps: translucent ring, tiny windows, client rounder than the config"
status: "open"
area: "core"
priority: "low"
blocked: "csd-backdrop-truncated-by-strip"
---

# Ring annulus test gaps: translucent ring, tiny windows, client rounder than the config

Filed 2026-11-03 from the review of PR #882 (the CSD backdrop no longer
sticks out past the ring's outer arc). Serves **daily-drive**: the focus
ring's corners are what a user looks at most, and they have regressed six
times because earlier tests pinned structure, not the pixels you see.

## The gap

PR #882 added `rounded/tests/annulus.rs`, which measures the ring's radial
thickness at every angle across radii, thicknesses, scales and both client
kinds. It does not yet cover:

1. **A translucent ring color.** The backdrop and ring overlap in the
   strips; a translucent color could show the overlap as a darker band.
2. **A window smaller than twice the radius.** Corners meet; the strip
   canvases and the backdrop side clamp differ from the normal case.
3. **A client rounder than the configured radius** (client radius 24 against
   `corner_radius` 8 or 12), at scales 1, 1.25, 1.5 and 2. This is the
   backdrop's original job (`client-rounded-corners-vs-ring-done.md`): the
   crescent between the client's own curve and the ring must stay
   ring-colored, with no background sliver and nothing outside the outer
   arc. The review of #882 argued this structurally (the new backdrop side
   equals the old extent) but did not run it.

## Status (2026-11-03)

All three cases are written in `rounded/tests/annulus.rs` and verified on
pixman and the GPU tier. The square-client ones run: a translucent ring and
a window smaller than twice the radius, bound and unbound, across scales.
They catch an unclipped backdrop (the wedge) but not a removed or shrunk one,
because an opaque client hides the backdrop. The three crescent ones (a
translucent ring behind a rounder client, a tiny rounder client, and case 3
itself at scales 1, 1.25, 1.5 and 2) catch removed, shrunk and unclipped
backdrops, and they **found a real bug**: the backdrop is cut off at the ring
strip's height, [`csd-backdrop-truncated-by-strip.md`](./csd-backdrop-truncated-by-strip.md).
They are `#[ignore]`d until that lands; with its one-line fix applied all
eight annulus tests pass on both renderers. Resolve this entry when they are
un-ignored.

## What to do

Add the three cases to the annulus suite with a client that draws its own
rounded alpha corners (an `xdg-decoration`-less test client), on pixman and
on the GPU renderer (`SCOOT_TEST_RENDERER=gpu`). Prove each fails if the
backdrop is shrunk or removed.

## Not in this ticket

Any change to the ring's geometry or the backdrop's mechanism.
