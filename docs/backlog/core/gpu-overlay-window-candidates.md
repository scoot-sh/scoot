---
title: "GPU scanout: let windows ride overlay planes (ScanoutCandidate marking + capture contract)"
status: "open"
area: "core"
priority: "low"
blocked: null
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

This was blocked because nothing reachable could verify it. Marking
windows without a live overlay assignment to prove captures still correct
would have shipped an unverified capture-correctness risk.

## Unblocked 2026-09-25: Asahi's `apple,dcp` has one overlay plane

The plane inventory is now recorded (`Asahi.md`, Test 5 results, from
`drm_info` on the Apple M2). CRTC 45 exposes exactly **one primary (35),
one overlay (40) and no cursor plane**. The overlay:

- has a fixed `zpos` of 1, so it sits above the primary. It can only be an
  overlay, never an underlay;
- accepts `LINEAR` only, the same as the primary;
- takes `AR30 AR24 AB24 NV12 NV16 NV24 P010 P210` and **no opaque `X`
  formats**. An `XR24`/`XR30` buffer cannot ride it; an `AR24` one can.

So verification is now possible on real hardware, with narrow bounds that
shape the design:

- At most one window per frame, and only one whose buffer is `LINEAR` in
  one of those formats. Mesa's AGX clients allocate
  `APPLE_GPU_TILED_COMPRESSED` by default (`Asahi.md`, Test 6). A candidate
  would therefore also need a per-surface tranche steering it to `LINEAR`
  in an overlay format, the way the scanout tranche does for the primary
  (Mesa was seen to follow that tranche).
- The same single overlay is the only place the cursor could go on this
  hardware, because there is no cursor plane. Today the cursor never lands
  there: Smithay allows `Kind::Cursor` on an overlay, but a memory buffer
  has no framebuffer to export. Marking windows competes for the plane
  with any future cursor-on-overlay work (`Asahi.md`, Test 5: the
  composited cursor is what blocks primary-direct here). Decide the
  priority between the two before building either.
- **No client observed on this machine could ride it.** Every client
  seen used a fourcc the overlay rejects: `XR30` (es2gears, mpv's GL
  output) and `XR24` (vkcube). Verifying this ticket therefore needs a
  purpose-built client that renders `AR24` at `LINEAR`, for example a
  small GBM or dumb-buffer test client like the dev VM's, run in a tiled
  window.
- The capture contract in the first paragraph still applies unchanged.
  Verify it on this machine: debugfs `dri/2/state` shows which fb plane 40
  holds, and a capture must still contain that window.

Priority stays low: the case it serves (a video in a non-fullscreen
window) is the rarer one.
