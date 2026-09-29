---
title: "Appearance: floating or flush, rounded corners, opacity, separators, state colors"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "config-cli-and-reload"
milestone: "M3"
---

# Appearance

Filed 2026-09-29. Serves **daily-drive**. Beauty is second to speed and
correctness (`CLAUDE.md`): every knob here is measured, and one that costs
real CPU or memory is off by default or cut.

## What to build

- **Placement**: flush to the edge, or floating with a margin and corner
  radius. Floating needs an ARGB surface and a transparent margin; measure
  the extra fill cost and the damage area against flush.
- **Margins are first-class, and use the protocol, not transparent pixels.**
  `margin` is per side (CSS-style shorthand: one, two or four values) and is sent as
  the layer surface's own margin (`zwlr_layer_surface_v1.set_margin`) so the
  compositor leaves that space empty. The surface is then exactly the bar's size:
  no oversized ARGB buffer, no transparent margin area, and no clicks swallowed by
  invisible pixels. Drawing a transparent border into a bigger surface is the
  wrong way to do this and is not done.
  - **Inner spacing**: bar `padding`, module `spacing`, and a per-module `margin`.
  - **Match scoot's gaps**: the default margin equals scoot's window gap so a
    floating bar lines up with the tiling; document the two settings to change
    together.
  - **Per-output override** of every one of these belongs to
    [multi-output](multi-output.md).
- **Exclusive zone with a margin**: the reserved strip must be the bar plus the
  margin on its anchored edge, or windows sit too close or too far. Whether
  scoot counts that margin toward the zone is checked against the pinned
  Smithay's `arrange` and pinned by a headless test (a bar with a margin, a window
  placed beside it, the gap measured on a screenshot); do not assume it.
- **Regions**: with protocol margins the surface has no transparent margin, so the
  only transparent pixels are rounded corners. A flush, opaque bar declares an
  **opaque region** (scoot can skip blending under it); a translucent one cannot,
  and its blend cost is part of what is measured. Set the **input region** to the
  bar's rounded shape only if scoot honors it (scoot's `state.rs` handles input
  regions; verify end to end), otherwise the corners are just clickable.
- **Shape**: bar radius, module padding and spacing, optional separators,
  the active-workspace pill's radius and inset. Corners are analytic
  coverage, no supersampling; edge pixels cached, not recomputed per frame.
  The rounded rectangle lands with **snapshot tests** at 1x and a
  fractional scale (a scene in `crates/scootbar/src/snapshots/tests.rs`;
  [testing](../testing.md#snapshots)): [testing-and-ci](resolved/testing-and-ci-done.md)
  asked for them, but the canvas had no rounded shape to snapshot yet.
- **Opacity**: bar background alpha in the ARGB buffer (premultiplied). No
  blur, no gradients, no shadows.
- **State classes** (`normal`, `warn`, `urgent`, `muted`, hover) map to color
  tokens; a module cannot invent colors, which keeps a theme source (Stylix)
  in control. Per-module overrides are a token per state, not free-form styles.
- **No animation by default.** A workspace-pill transition or hover fade means
  frame callbacks while it runs; if built, it is opt-in, bounded (a few
  frames), and back to zero wakeups the moment it ends. Measured or dropped.
- Per-output overrides (a taller bar on a HiDPI output) belong to
  [multi-output](multi-output.md).

## Coordination with scoot

The compositor's focus ring and rounded window corners are drawn by scoot; the
bar shares no code or config with them. Keep its default radius and colors
visually compatible, and document the tokens users would set to match.

## Done when

The default look is good with no config, each option is documented in the
same PR, and the flush and floating variants have measured costs published
with the [resource ratchet](lightest.md).
