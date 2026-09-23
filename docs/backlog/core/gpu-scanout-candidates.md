---
title: "GPU scanout: mark eligible window surfaces as scanout candidates, with scanout-tranche dmabuf feedback"
status: "open"
area: "core"
priority: "high"
blocked: "gpu-direct-scanout-exporter must land first"
---

# GPU scanout: scanout candidates + per-surface scanout feedback

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**.
Depends on [the exporter widening](./gpu-direct-scanout-exporter.md).

## What is missing

Every window surface element is built `Kind::Unspecified`
(`render/elements.rs`, both call sites; `decorations.rs`), only cursor
elements are `Kind::Cursor`, and `Rounded` forwards its inner kind. Smithay's
overlay and primary-direct assignment only consider `ScanoutCandidate` (and
`Cursor`) elements, so even with the exporter widened no window can leave
the composited swapchain.

And a client cannot know which buffers would be scanout-able: the
`zwp_linux_dmabuf_v1` feedback today has one renderer tranche. Smithay's
pattern (anvil at the pinned rev) is per-surface feedback with a
scanout tranche built from the plane's formats, sent when a surface becomes
a candidate.

## What to do

- Decide the eligibility rule and write it down: the obvious first cut is a
  window that covers the whole output (fullscreen or sole full-size column),
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
