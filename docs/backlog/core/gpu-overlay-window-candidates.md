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

**Update 2026-09-29 (`Asahi.md`, Test 14):** on kernel 7.1.13 the CRTC
exposes **two** overlays (plane 40 zpos 1, plane 45 zpos 2, same `LINEAR`
formats, still no cursor plane). So a window and a future cursor-on-overlay
need not fight for one plane there. Nothing in this ticket can be measured
without implementing the marking and the capture contract.

**Update 2026-10-06 (`Asahi.md`, Test 17):** the drawn cursor now rides
an overlay on this hardware (`render/cursor_plane.rs`). It is offered the
planes first, as the front-most element, and takes the topmost that passes
(plane 45, zpos 2, every time it was seen), which leaves plane 40 below it
for a window candidate. On a CRTC with a single overlay the cursor yields
to a marked window candidate instead (`cursor_rides_overlay` gates on
`overlay_planes >= 2` or no candidate): the cursor would otherwise take the
only overlay ahead of the window, and a tiled video that rode it before
would composite.

Priority stays low: the case it serves (a video in a non-fullscreen
window) is the rarer one.

## Implementation exists, live proof blocked 2026-09-28

PR `feat/overlay-window-candidates` implements all of the above (capture
contract first, priority decision, marking, tranche steering). Dev-VM
verification is green (workspace + `gpu-scanout` nextest, clippy, fmt,
smoke, benchmarks). The live overlay leg is **blocked**: DP-1 is dark and
nothing remote wakes it.

- `drm_info` (2026-09-28, kernel 7.1.13): DP-1 `disconnected`, 0 modes,
  empty EDID; eDP-1 `connected` (2560x1600). Force file reads
  `unspecified` (pristine).
- Tried, each verified by read-back: `echo on | sudo tee
  /sys/kernel/debug/dri/2/DP-1/force` (latched `on`), three synthetic
  `udevadm trigger --subsystem-match=drm --action=change` reprobes with
  3-12 s waits (status stayed `disconnected`, modes 0), `dpms` (already
  `On`; writing it is refused), `dmesg` (no link/HPD activity after any
  trigger), and a `sudo reboot` (remote-safe; cleared the force latch back
  to `unspecified`, but the boot-time probe did not wake the monitor
  either). The monitor has been undriven for ~2 days (standby since
  2026-09-26); it needs a power-button press. eDP-1 was not touched.
- Ready for a hands run: `~/fx/overlay/` on the box holds the `gpu-scanout`
  build of this branch (`overlay-scoot-gpu` ->
  `/nix/store/4fzgsw8a6mzamsralm4qbabspl4bsn3b-scoot-gpu-0.1.0`,
  `overlay-scootctl` -> `...-scootctl-0.1.0`; rebuilt after checkout, same
  hash twice, so it carries this branch) and `t11.sh`, which fails loudly
  unless DP-1 is `connected` with modes and otherwise runs the whole leg
  (tiled foot to DP-1, pointer parked, debugfs snapshots, screenshots with
  and without the cursor over the window).

## Premise update 2026-09-28: two overlays per CRTC, not one

The inventory above was recorded on kernel 7.1.5. On the 7.1.13
fairydust kernel now running, each CRTC has **one primary, two overlays
(zpos 1 and 2, fixed) and no cursor plane** (`drm_info`: CRTC 0/eDP-1
planes 35/40/45, CRTC 1/DP-1 planes 53/58/63; overlay formats unchanged:
`AR30 AR24 AB24 NV12 NV16 NV24 P010 P210` at `LINEAR`, no `X` fourccs).
The implementation marks at most one window regardless, and Smithay offers
the topmost compatible overlay first (front-to-back order), so a lone
candidate takes zpos 2 (plane 63 on DP-1, 45 on eDP-1) -- expect that,
not plane 40/58, in `dri/2/state`. The window-vs-cursor priority is
 unchanged by the second plane (the cursor is tried first by z-order), with
 room for both once the cursor can ride.

 ## Live proof 2026-10-02: the window rode plane 63

 DP-1 came back (`connected`, 21 modes, kernel 7.1.13, force
 `unspecified`). One careful run (`/tmp/ovl313.sh`, artifacts in
 `/tmp/ovl313/`: compositor log, three `dri/2/state` dumps, three
 screenshots, outputs/windows JSON):

 - Foot could not prove this leg: it allocates tiled buffers at spawn and
   Mesa never reallocates to `LINEAR` on tranche arrival, so every KMS
   attempt failed `could not import framebuffer` (5 in the ovl312 log).
   The run used `ovl-linear` instead, a throwaway client that waits for a
   tranche containing `(AR24, LINEAR)` and only then allocates a DRM dumb
   buffer (always `LINEAR`) sized to its configure (source: `/tmp/ovlclient`
   on the box).
 - Riding: plane 63 (zpos 2, DP-1) `fb=82`, `AR24` modifier `0x0`,
   622x696 pitch 2496 — the client's exact buffer. Resized: plane 63
   `fb=77`, 833x696 pitch 3392. Cursor over the window: plane 63 `fb=0`
   (ride ends on overlap, as designed).
 - `could not import framebuffer` count for the whole run: 0. The pick
   marks `(WindowId(1), …, true, false)`; the client log shows
   `OVL_TRANCHE_AR24_LINEAR` before every `OVL_READY` (tranche-before-alloc
   ordering held across all three sizes).
 - Screenshots parsed and pixel-checked locally: the pattern's white core,
   magenta field and lime border exact in the riding, resized *and* cursor
   shots — the capture contract holds in every phase.
 - The riding build (`/nix/store/89lgs9a4…-scoot-gpu-0.1.0`) is this branch
   plus only the two temporary `overlay pick` debug lines (verified by
   diffing its nix source against the branch head; `/tmp/ovl310-pick-debug.patch`
   is that delta, stale by one `_ => "other"` arm). Box left pristine
   (VT1, eDP untouched, `/dev/dri/card2` perms restored after a transient
   `chmod 666` the spawned client needed to `CREATE_DUMB`).
