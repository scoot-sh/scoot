---
title: "GPU scanout: let windows ride overlay planes (ScanoutCandidate marking + capture contract)"
status: "open"
area: "core"
priority: "low"
blocked: "needs hardware with overlay planes — virtio has none; Asahi's apple,dcp plane inventory is unrecorded (Asahi.md)"
---

# Windows on overlay planes

Split 2026-09-23 out of [scanout candidates](../resolved/gpu-scanout-candidates-done.md)
(coordinator). Serves **daily-drive** (e.g. a video in a non-fullscreen
window scanned out on an overlay).

No window is `Kind::ScanoutCandidate`, so none rides an overlay plane.
Before marking anything, the capture contract must be extended:
`Captures::note_direct` fires only for primary-direct today
(`ScanoutFrame::primary_direct`); a window on an overlay is equally absent
from the swapchain slot, so a capture would silently miss it. Share the
rounded/translucent refusals in `render::primary_direct` rather than copying
them.

Blocked because it cannot be verified anywhere reachable: marking windows
without a live overlay assignment to prove captures still correct would
ship an unverified capture-correctness risk. Unblock by recording the
Asahi plane inventory (`drm_info`) or finding a VM/device with overlays.
