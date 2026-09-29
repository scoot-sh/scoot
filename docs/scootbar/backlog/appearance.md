---
title: "Appearance: floating or flush, rounded corners, opacity, separators, state colors"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock, config-cli-and-reload"
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
- **Shape**: bar radius, module padding and spacing, optional separators,
  the active-workspace pill's radius and inset. Corners are analytic
  coverage, no supersampling; edge pixels cached, not recomputed per frame.
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
