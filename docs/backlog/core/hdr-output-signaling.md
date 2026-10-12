---
title: "HDR connector signaling: HDR_OUTPUT_METADATA, Colorspace, max bpc, PQ output"
status: "open"
area: "core"
priority: "low"
blocked: "needs an HDR-capable display plus a driver attaching HDR_OUTPUT_METADATA; needs a Smithay fork change per hdr-color-management findings"
---

# HDR connector signaling: HDR_OUTPUT_METADATA, Colorspace, max bpc, PQ output

Filed 2026-10-11. Serves **daily-drive**: this is the stage that lights
up an HDR panel (metadata, colorspace, EOTF); not needed for computer use.

Stage 4 of the plan in [hdr-color-management](../resolved/hdr-color-management-done.md).

## The gap

Nothing drives the connector side: Smithay's `DrmCompositor` atomic
commit writes only `CRTC_ID` for connectors (pinned fork
`src/backend/drm/surface/atomic.rs:1176-1182`) and exposes no
`HDR_OUTPUT_METADATA` / `Colorspace` / `max bpc` write API. The GPU
pipeline (PQ EOTF + gamut conversion, e.g. scoot-side custom render
elements on HDR outputs) and SDR-brightness handling also belong here.
KWin fills mastering metadata where Mutter leaves zeros (research
ticket) — follow the KWin shape, not the Mutter one.

## What to do

Write the blob/enum/range connector properties on HDR-capable outputs
behind an opt-in key, output PQ EOTF with rec.2020 signaling, convert or
map SDR content with a stated brightness policy, and report the state
honestly. **This stage needs a Smithay fork change** (small setters on
`DrmCompositor` after the `use_vrr` precedent, with the `dev/forks.md`
alternatives-considered entry the fork rule requires): raw atomic
commits from scoot would race Smithay's, and an in-tree `DrmCompositor`
copy is ~4400 lines. Blocked on HDR hardware (HDR10 sink + driver with
`HDR_OUTPUT_METADATA`) and on stages 1-3.

## Not in this ticket

Probe, scanout depth and protocol dispatch (stages 1-3). No CPU-tier
work: the CPU tier never signals HDR.
