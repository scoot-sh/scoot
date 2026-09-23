---
title: "GPU scanout: primary-direct is gated on a swapchain format match no client buffer meets"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# GPU scanout: the primary-direct format gate

Filed 2026-09-22 by the exporter widening
([resolved](../resolved/gpu-direct-scanout-exporter-done.md)), which found this
gate where it expected direct scanout. Serves **daily-drive** (zero-copy
fullscreen video/games). This, not candidate marking, is what stands between
the tree and primary-direct scanout on any device.

## What is missing

Smithay's `try_assign_primary_plane` (pinned rev) has no element-kind check.
Its gate, unless `ALLOW_PRIMARY_PLANE_SCANOUT_ANY` is set, is
`slot.format() != element_config.properties.format` -- a whole-`Format`
comparison, fourcc *and* modifier, between the swapchain slot and the
framebuffer the exporter made from the client buffer. For every buffer a
client can send scoot today it is unequal twice:

- **fourcc**: the primary path exports with `allow_opaque_fallback`, so the
  client framebuffer is always the opaque variant (`XR24`), while the
  swapchain is `AR24` wherever the plane takes it (`COLOR_FORMATS` in
  `tty/scanout.rs` lists `Argb8888` first).
- **modifier**: `zwp_linux_dmabuf_v1` offers only `LINEAR` (`dmabuf.rs`),
  and Smithay refuses to export implicit-modifier client buffers; the
  swapchain on a plane without `IN_FORMATS` is allocated implicitly
  (`Modifier::Invalid`). The dev VM's virtio-gpu is that case (measured:
  `Testing Formats: [AR24, Invalid]`), so reordering `COLOR_FORMATS` alone
  would not match there.

## What is known to work once the gate is lifted

With `FRAME_FLAGS |= ALLOW_PRIMARY_PLANE_SCANOUT_ANY` as an uncommitted
experiment on the dev VM, a full-output card0 dumb-buffer client (card0
allocations, dumb or GBM `LINEAR`, are the only provenance that imports
there -- `renderD128` allocation is refused and `kms_swrast` refuses udmabuf
imports) went primary-direct frame after frame
(KMS plane 33 on the client's `XR24`/`LINEAR` fb), and the PR #218 capture
fix fired live: each capture forced a composite frame and read current
pixels, a capture while VT-switched away refused loudly, and it recovered on
the switch back. A 782x976 window at its column offset failed the atomic `TEST`
(consistent with virtio's primary plane having to cover the CRTC -- not
verified further) and composited, cached as failed. Forced frames
already drop both primary bits (`composite_only`), so adding `ANY` would not
let a forced capture frame go direct.

## What to decide

- **`ANY` vs a format change.** `ANY` skips the format comparison entirely
  and leaves the atomic `TEST` as the only judge -- the simplest lift, and
  the one measured working. Its risk is the reason it is a separate bit:
  it hands KMS buffers whose format differs from the swapchain's, which a
  driver may accept and display with the wrong alpha/colour interpretation
  rather than refuse. Alternatively put `Xrgb8888` first in `COLOR_FORMATS`
  *and* allocate the swapchain with an explicit `LINEAR` where the plane
  allows it -- narrower, but it changes every composited frame's format and
  must be measured (CPU, captures byte-identical, `read_back`'s ARGB
  assumption).
- **Which elements may be tried.** Primary-direct needs the bottom visible
  element with everything above it on planes, and either a black/transparent
  clear colour or a whole-output opaque element. With a black
  `background_color` *any* bottom window qualifies -- and `Rounded` forwards
  `underlying_storage`, so a rounded window taken direct loses its corner
  clip. Decide whether the eligibility rule lives here (e.g. only when the
  element spans the output) or rides the
  [candidates](./gpu-scanout-candidates.md) rule.
- **Cost on the stale-capture path.** Each forced frame costs the direct
  client a framebuffer re-export on its next frame (the composite-only frame
  never runs `element_config` for it, so Smithay's per-element framebuffer
  cache is not carried over: measured 6 exports over a 5-second run with two
  captures vs 2 without) and reallocates the swapchain
  (`invalidate_scanout`). Fine for screenshots; measure before a
  screencopy client at 60 Hz rides it.

## Evidence expected

Live primary-direct on whatever hardware the chosen lift reaches, with
commit health (KMS state naming the client fb), captures byte-correct
through direct frames, and the paused-capture refusal. The experiment
recipe (black background, no ring, radius 0, one full-output column, a
card0 dumb-buffer client) is in the exporter ticket's PR.
