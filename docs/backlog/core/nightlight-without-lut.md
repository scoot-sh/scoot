---
title: "Night light on outputs with no gamma LUT (Apple DCP): warm the image in scoot's renderer"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Night light on outputs with no gamma LUT (Apple DCP): warm the image in scoot's renderer

Filed 2026-10-06 from the PR #472 live proof. Serves **daily-drive**: the
maintainer's own Apple-silicon laptop can't get a night light today.

## The gap

`wlr-gamma-control-v1` sets the CRTC's `GAMMA_LUT`. On the Asahi M2, the
Apple DCP display controller exposes no `GAMMA_LUT` property (`drm_info`
as root shows none), so every `set_gamma` is refused with `failed()`. So
no gamma-based night light can warm the screen on that machine: wlsunset,
gammastep and hyprsunset all fail. #472 documents this as a symptom; it
doesn't fix it.

## What to do

Research, then implement if it fits:

- **Can scoot apply a color transform in its own render path?**
  - On the GPU renderer: a per-output shader pass or a color matrix
    applied at composition. It costs a full-frame pass, so measure it.
    Direct scanout of a fullscreen client would have to be disabled
    while warming, unless a plane-level CTM exists.
  - On pixman: a per-pixel transform on damaged regions only. Measure the
    CPU cost.
  - Does the DCP expose a CTM (color matrix) instead of a LUT, or does
    Asahi plan one? Check `drm_info` and the kernel driver sources at the
    running version.
- **Interface:**
  - If it fits, feed it from the same `wlr-gamma-control-v1` ramps.
    scoot would accept a control on a LUT-less output and apply the ramp
    in software, so existing daemons work unchanged.
  - Or offer an IPC/config knob instead. Prefer the protocol path if the
    cost is acceptable.
- **Constraints:**
  - Battery and per-frame cost: no allocation per frame (CLAUDE.md), and
    benchmark before and after. Damage tracking must still limit redraws.
  - Screenshots stay pre-transform, as now.
  - The lock screen is warmed too.

## Not in this ticket

Full color management / ICC.
