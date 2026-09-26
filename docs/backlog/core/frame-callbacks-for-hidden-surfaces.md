---
title: "Withhold frame callbacks from layer surfaces nobody can see"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Withhold frame callbacks from layer surfaces nobody can see

Found in review of the scootbg plan (PR #264, 2026-09-26).

After each rendered frame, scoot sends `wl_surface.frame` to every mapped
layer surface on the output (the post-render loop in
`crates/scoot/src/compositor/headless.rs`, "Sent to every mapped layer
surface on this output"). There is no occlusion check. A client that
paces animation by frame callbacks, which is the protocol's intended way
to stop drawing when unseen, therefore never learns it is covered: an
animated wallpaper under a fullscreen video keeps decoding and blending
at the video's frame rate, on the CPU under pixman.

This matters for scootbg's animated wallpapers
([`crates/scootbg/backlog/animated-images.md`](../../../crates/scootbg/backlog/animated-images.md))
and for any other animated background or bottom-layer client.

## Constraints on a fix

- The current behaviour is deliberate for a reason that still holds: a
  client may ask for a callback before its first attach, and withholding
  it would stall the frame that unsticks it. So only a surface that has
  already committed a buffer, and is fully covered, is a candidate.
- "Fully covered" means covered by opaque content on that output (a
  fullscreen window with an opaque region or opaque format, or an
  opaque-format window spanning the output), computed from what the frame
  already knows, not with a per-frame allocation.
- The callback resumes on the first frame where any of the surface is
  visible again, and a withheld callback is never lost (the client gets
  the next one).
- Measure: CPU of an animated background client under a fullscreen opaque
  window, before and after, plus a check that the compositor's own frame
  time does not grow.
