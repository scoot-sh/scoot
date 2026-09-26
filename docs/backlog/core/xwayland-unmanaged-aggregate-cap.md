---
title: "Multi-X-client aggregate unmanaged-window pressure: N clients × 128 menus per motion and per frame"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Multi-X-client aggregate unmanaged-window pressure

Filed 2026-09-26 from the PR #258 review. The per-X-client unmanaged cap
(`xwayland-unmanaged-pressure-cap.md`, resolved by #258) bounds each
client at 128 hit-test walks and draws — but the `x11_unmanaged` list is
global, so N X clients give N×128 per pointer motion and per frame with
nobody tripping a cap. Same class as `popup-aggregate-pressure-cap.md`
for popups (reference it); linear, not quadratic, and behind the
session-owned X socket, hence low. Fix shape open: a global unmanaged
bound or aggregate accounting — do not implement here.
