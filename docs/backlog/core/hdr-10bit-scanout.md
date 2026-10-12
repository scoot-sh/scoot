---
title: "Opt-in 10-bit scanout format on the GPU tier (SDR content unchanged)"
status: "open"
area: "core"
priority: "low"
blocked: "needs a 10-bit-capable display to verify the positive path; negative path verifiable anywhere"
---

# Opt-in 10-bit scanout format on the GPU tier (SDR content unchanged)

Filed 2026-10-11. Serves **daily-drive**: 10-bit scanout is the
banding-free base HDR needs; not needed for computer use.

Stage 2 of the plan in [hdr-color-management](../resolved/hdr-color-management-done.md).

## The gap

The GPU scanout swapchain is 8-bit only: `COLOR_FORMATS = [Argb8888,
Xrgb8888]` in `crates/scoot/src/compositor/tty/scanout.rs:443`, while the
Smithay GLES3 renderer already imports 10-bit and FP16 memory formats
(`Abgr/Xbgr2101010`, `Abgr/Xbgr16161616f`). Nothing offers a deeper
scanout format even where the panel and plane support one.

## What to do

An opt-in `[output]` key (name to taste, default stays 8-bit) selecting a
10-bit swapchain format through the already-public
`DrmCompositor::with_format` / `set_format`, honoured only where the
plane intersects, with the refusal naming why elsewhere. SDR content
renders unchanged (same pixels, deeper container); no EOTF or metadata
work here. Negative path (key refused on 8-bit-only output) verifiable
without HDR hardware. No fork change: the seam is public.

## Not in this ticket

Protocol work (stage 3, `hdr-color-protocol`), connector signaling
(stage 4, `hdr-output-signaling`).
