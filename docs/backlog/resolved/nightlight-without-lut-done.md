---
title: "Night light on outputs with no gamma LUT (Apple DCP): warm the image in scoot's renderer"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (PR #473, 2026-10-06)

Phase 1 found the DCP exposes a per-CRTC `CTM` blob (live `drm_info` as
root: `CTM` on both CRTCs, gamma size 0, no plane color props), wired to
real hardware in the Asahi driver (`drm_crtc_enable_color_mgmt(0, true,
0)`; `iomfb_flush` pushes the blob through IOMFB `set_matrix`; KWin
Night Color proves the pipeline). So no software render path was built:
scoot applies the `wlr-gamma-control-v1` ramp's white endpoints as a
diagonal S31.32 matrix in one synchronous atomic commit per `set_gamma`
(`crates/scoot/src/compositor/tty/ctm.rs`; LUT still wins where both
exist). Exact at the daemons' default gamma; zero per-frame cost, direct
scanout unaffected, screenshots pre-transform by construction, lock
screen warmed automatically. 8 unit tests green; full suite
(2139+4 passed), clippy (default + `gpu-scanout`), fmt, headless smoke
(23 ok), docs-site build all green on the M2. Live `--tty` proof on the
panel outstanding (seat0 held throughout); the ioctl half is reviewed
against the pinned `drm` 0.14.1 API and the driver source.
