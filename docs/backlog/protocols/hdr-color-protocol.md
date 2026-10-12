---
title: "Server-side wp-color-management-v1 and color-representation-v1 dispatch"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Server-side wp-color-management-v1 and color-representation-v1 dispatch

Filed 2026-10-11. Serves **daily-drive**: without the protocol, HDR
clients (mpv, Firefox) have nothing to negotiate with; not needed for
computer use.

Stage 3 of the plan in
[hdr-color-management-done](../resolved/hdr-color-management-done.md).

## The gap

scoot advertises no `wp_color_manager_v1` (nothing in
`crates/scoot/src`, nothing on the protocols page), and the pinned
Smithay fork has no server module for it (upstream only the untested
#1143 draft). An HDR client therefore cannot learn output color
properties or tag its surfaces, and falls back to SDR silently.

## What to do

Hand-written scoot-side dispatch on the generated `wayland-protocols`
0.32.13 staging server types (XML and `wp::color_management` module
already vendored; interfaces at version 3): accept surface image
descriptions, report per-output descriptions derived from EDID, plus the
`color-representation-v1` companion for YUV formats. Advertise only on
the GPU tier (CPU tier policy: refuse by not advertising — see the
research ticket). Re-check Smithay PR #1143 at implementation time and
repin rather than duplicate if it ever lands (`smithay-fork-repin`).
No fork change: there is no Smithay module to wrap and nothing to copy.

## Not in this ticket

Scanout depth (stage 2), connector signaling and EOTF output (stage 4).
The CPU tier never advertises this (policy in the research ticket).
