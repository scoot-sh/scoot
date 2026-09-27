---
title: "XWayland: X windows are not scale-aware (drawn at scale 1, upscaled)"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: X windows are not scale-aware

Split out of [XWayland support](../resolved/xwayland-support-done.md) when
its Phases 5–7 closed it (2026-09-27). Serves daily use on HiDPI screens
(the Asahi M2 runs `[output] scale = 2`); irrelevant at scale 1, which is
every headless/webtop/VM session.

## Today

X clients draw at scale 1 and scoot upscales their buffers like any
scale-unaware client (`compositor/xwayland/manage.rs`'s module doc,
"Scale"). At `[output] scale = 2` an X app is the right size but blurry;
at a fractional scale it is upscaled by the pixman/GLES sampler.

## Shape of the fix

The approach other Smithay compositors use, all available at the pinned
fork rev with no change:

- Set the XWayland client's compositor client scale
  (`CompositorClientState::set_client_scale`, reached through
  `XWaylandClientData`) to the output scale. Smithay's XWM already divides
  X geometry by `client_scale` (`xwm/mod.rs`: configure requests, window
  geometry), and its seat and `wl_output` code scale what XWayland is told,
  so XWayland renders at native resolution and scoot treats its buffers as
  scale N.
- Tell X toolkits to draw at that scale, or they render tiny: `Xft.dpi`
  in the X server's resource database (`xrdb`-equivalent, over the WM's own
  connection) and/or `GDK_SCALE` / `QT_SCALE_FACTOR` in the environment
  `State::spawn` hands X children.
- Re-apply on an `[output] scale` reload, and decide what a fractional scale
  rounds to (X toolkits scale by integers; a 1.5 session would round the X
  side up to 2 and let scoot downscale, or stay at 1).

## Sizing

- **Files:** `compositor/xwayland/mod.rs` (set the scale at `READY`),
  `compositor/output_scale.rs` / reload path (re-apply on change),
  `compositor/xwayland/unmanaged.rs` (`rect_of` mixes `last_configure()`
  logical coordinates with `bbox()` sizes -- re-check both under a client
  scale), `compositor/xwayland/manage.rs` (the `INT16`/`CARD16` clamp is on
  X-side, i.e. scaled, values), `State::spawn`'s X environment, pointer and
  drag coordinate paths through the live suites, docs. Probably 200–400
  lines with tests; the live suites need a scaled fixture
  (`Harness::headless_scaled` exists).
- **Fork change:** none expected. Verify first that everything Smithay
  scales by `client_scale` is scaled consistently at the pinned rev -- the
  DnD and selection bridges (fork commits) and the XDND proxy placement are
  the likely places for an unscaled coordinate.
- **Risk:** medium-high. It touches every X coordinate path (hit-testing,
  override-redirect placement, the drag proxy, floating placement against
  `USPosition`), and the per-output work is simplified only because scoot
  has one scale for every screen; per-output scale
  ([`per-output-scale-mode`](../core/per-output-scale-mode.md)) would make
  this a per-screen problem that X cannot express.
