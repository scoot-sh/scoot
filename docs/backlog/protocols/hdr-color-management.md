---
title: "Research: HDR and color management (wp-color-management-v1, DRM HDR metadata)"
status: "research"
area: "protocols"
priority: "research"
blocked: null
---

# Research: HDR and color management (wp-color-management-v1, DRM HDR metadata)

Filed 2026-10-11. Serves **daily-drive**: HDR monitors and HDR video are common on desktops now; not needed for computer use.

## The gap

scoot has no HDR and no color management (checked 2026-10-11): no
`wp_color_management_v1` in `crates/scoot/src`, none listed on the protocols
page, and `gpu-vrr` records it as "a different order of work, not filed". The
pinned Smithay fork (`fdf424d6`) has no color-management protocol module and
no DRM `HDR_OUTPUT_METADATA` / `Colorspace` connector-property helpers (grep of
`src/` finds neither). The CPU tier composites 8-bit sRGB in pixman.

## What to do

A research pass, no code:

- Which protocol: `wp-color-management-v1` is the standard, so per CLAUDE.md
  it is the target. Check its state and which clients use it (mpv, Firefox,
  KWin and Mutter as references).
- What it needs: HDR metadata and colorspace on the DRM connector, a 10-bit or
  FP16 scanout format, and a GPU-tier color pipeline (transfer function and
  gamut conversion). Which of these scoot can do scoot-side without a
  Smithay fork change (forks are the last resort; see `dev/forks.md`).
- The CPU tier: say plainly that HDR is a GPU-tier feature, and what the CPU
  tier does with an HDR client (tone-map to SDR, or refuse). CPU and GPU stay
  equal tiers for what both can do.
- Hardware: which monitor and GPU could test it (the Asahi M2's panel and
  external displays; the VMs cannot).
- Output: a scoped plan with stages, or a decision not to do it yet.

## Not in this ticket

Implementation, and `wp_tearing_control_v1` (see `gpu-vrr`).
