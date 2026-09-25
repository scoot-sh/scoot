---
title: "Variable refresh rate (adaptive sync) on the GPU scanout tier"
status: "open"
area: "core"
priority: "low"
blocked: "needs a VRR-capable display; virtio-gpu has none, and the Asahi eDP panel is confirmed not VRR-capable (2026-09-25)"
---

# Variable refresh rate on the GPU scanout tier

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**
(games/video on VRR monitors).

`wlr-output-management` reports `adaptive_sync` as always `disabled`
(`docs/protocols.md`: "scoot has no VRR support"). The pinned Smithay's
`DrmCompositor` has `vrr_supported(conn)` / `use_vrr` / `vrr_enabled`.

## What to do

An opt-in config key (`[output] vrr = "off" | "on"`, name to taste; default
off), honoured only on the GPU scanout tier where `vrr_supported` says so,
reported truthfully through output management, with the refusal naming why
elsewhere. Smithay skips VRR for cursor-only updates — check whether scoot's
frame scheduling needs anything else.

## Not in this ticket

`wp_tearing_control_v1`: the pinned Smithay rev does not carry it (checked
2026-09-22: no `tearing_control` in `src/`), so it would be hand-written;
file separately if a client need shows up. HDR / color management is a
different order of work and is not filed.

## Blocked

Needs a VRR-capable connector to verify the positive path. The negative
path (`vrr_supported` false → key refused/reported disabled) is verifiable
on the dev VM and can land first if split.

**Asahi checked 2026-09-25 (`Asahi.md`, Test 5 results): this panel is not
VRR-capable.** On the Apple M2's `eDP-1`, `drm_info` shows **no
`vrr_capable` property on the connector at all**. Its properties are
`EDID`, `DPMS`, `link-status`, `non-desktop`, `TILE` and `CRTC_ID`, and it
has one mode, `2560x1600@60`. The CRTC does carry `VRR_ENABLED` (range
0–1, set to 0), but with no capable connector that says nothing. niri,
driving the same panel, reports `Variable refresh rate: not supported`.
That makes this machine a place to exercise the negative path (the key
refused, and reported disabled, for a connector with no `vrr_capable`) on
real DCP hardware. It is not a place to verify the positive path. An
external VRR monitor on USB-C would be the next thing to try.
