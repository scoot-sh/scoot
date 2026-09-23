---
title: "GPU scanout: mark eligible window surfaces as scanout candidates, with scanout-tranche dmabuf feedback"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# GPU scanout: scanout candidates + per-surface scanout feedback

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**.
The [exporter widening](../resolved/gpu-direct-scanout-exporter-done.md) it
depended on has landed (`NodeFilter::All`), and so has
[client fullscreen](../resolved/client-fullscreen-done.md), which supplies the
whole-output window state the eligibility rule below needs:
`World::fullscreen_on(output)` (which window covers an output right now) and
`Placement::fullscreen`. Under a covering window the compositor already draws
no ring, no rounded clip and no `top` layer over it (`overlay` surfaces and
the cursor may still be above it).

## What is missing

Every window surface element is built `Kind::Unspecified`
(`render/elements.rs`, both call sites; `decorations.rs`), only cursor
elements are `Kind::Cursor`, and `Rounded` forwards its inner kind.
Smithay's **overlay** assignment only considers `ScanoutCandidate` (and
`Cursor`) elements, so no window can ride an overlay plane.

Primary-direct is gated differently: `try_assign_primary_plane` has no kind
check at all. It is tried only for the bottom visible element with nothing
composited above it, that element opaque and covering the whole output (or
a black/transparent clear colour), and then requires the client
framebuffer's whole `Format` to equal the swapchain's
([format gate](./gpu-primary-direct-format-gate.md)). On default config no
window meets the first half unless it is fullscreen (see
[client fullscreen](../resolved/client-fullscreen-done.md)): otherwise the
3 px focus ring is composited over the focused window, and the background is
not black. The
eligibility rule here is therefore also what decides which windows may be
*tried* for the primary -- one decision, shared with the format-gate ticket.

On the rounded clip: `Rounded` forwards `underlying_storage` (and `kind`),
so the element Smithay would scan out is the unclipped inner buffer -- the
corners are lost, not approximated.

And a client cannot know which buffers would be scanout-able: the
`zwp_linux_dmabuf_v1` feedback today has one renderer tranche. Smithay's
pattern (anvil at the pinned rev) is per-surface feedback with a
scanout tranche built from the plane's formats, sent when a surface becomes
a candidate.

## What to do

- Decide the eligibility rule and write it down: the obvious first cut is a
  window that covers the whole output (fullscreen, from
  [client fullscreen](../resolved/client-fullscreen-done.md), or a sole full-size column),
  with nothing composited over it (the focus ring hidden or not drawn over a
  fullscreen window),
  unrounded (a rounded window's clip means it cannot be scanned out whole —
  `corner_radius > 0` must exclude it unless the radius clip is provably a
  no-op), not under any other element, alpha 1.0, no transform/viewport the
  plane cannot express. Anything else stays `Unspecified`.
- Mark those `Kind::ScanoutCandidate`; everything that relies on captures
  reading the swapchain slot is already covered by the PR #218 force path —
  verify it still is once *windows*, not just the primary, can go direct
  (a window on an overlay plane is also absent from the swapchain slot:
  confirm `note_direct` covers overlay assignment of a window, not only
  primary-direct, and fix if not — that is a correctness blocker, not a
  follow-up).
- Per-surface dmabuf feedback with a scanout tranche for candidate
  surfaces, falling back to the default feedback when a surface stops
  being a candidate.

## Evidence expected

Live on whatever hardware goes direct (see the exporter ticket's hardware
note); if none is reachable from here, harness pins plus a clearly worded
README limitation. Captures must stay byte-correct with a candidate window
on screen — that is the property the whole plane series has protected.
