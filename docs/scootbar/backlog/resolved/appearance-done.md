---
title: "Appearance: floating or flush, rounded corners, opacity, separators, state colors"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-09-30"
---

# Appearance

Filed 2026-09-29. Serves **daily-drive**. Beauty is second to speed and
correctness (`CLAUDE.md`): every knob here is measured, and one that costs
real CPU or memory is off by default or cut.

## Landed and remaining

**Landed (first slice):** protocol margins and the exclusive zone with a
margin (`bar.margin`, pinned by `tests/bar.rs`, PR before this one); the
corner radius (`bar.radius`, analytic coverage, the corner table cached per
radius and scale in the scene); opacity (`bar.opacity`, a premultiplied
`ARGB8888` buffer only when a radius or opacity asks for one, else
`XRGB8888` as before); the opaque region (whole, a cross that leaves the
corner squares out, or none when translucent); snapshot scenes for the
rounded bar at 1x and 1.5x and a translucent one; a headless-scoot test of
each on screenshots; the measured fill costs in
[cli.md](../../cli.md#shape-and-opacity). Both options are file-only.

**Landed (second slice):**

- **The input region is the rounded shape.** Verified first: scoot honors
  `wl_surface.set_input_region` on a layer surface (`layer_surface_under`
  asks the surface tree, `state.rs`), checked end to end on a headless scoot
  (`tests/appearance.rs`: a click in the window's corner under a rounded
  bar reaches the window, on the flat part it does not, a square bar swallows
  both; with the region left unset the test fails). One rectangle per corner
  row, at most `2 x radius + 1`, set only when the size or radius changes
  (`src/region.rs`), exact where the plan was a cross of two rectangles, so
  the visible arc in each corner stays clickable.
- **Text is kept out of the corners** rather than clipped to them: the
  layout clears the bar's ends by `radius - padding / 2`, so the first and
  last module's ink and the pill are never in a corner square (decided over
  per-pixel clipping: it costs nothing in the paint and cannot leave a
  glyph half cut).
- **Per-module `margin`** (`[clock] margin`, `[workspaces] margin`) and
  **separators** (`[bar] separator`, a `dim` line in the gap, at most
  `spacing`), with bounded, loudly refused config. `bar.padding` and
  `bar.spacing` already existed (`--padding`, `--spacing`).
- **The workspaces pill**: `pill-shape` (`rect`, `pill`, `circle`),
  `pill-radius`, `pill-inset`; a circle widens into a pill around a
  two-digit number and stops short of its neighbours; the hit test follows
  the drawn pill. Snapshots at 1x and 1.5x for each, two digits included.
- **The example**: a `[bar]` snippet with `margin` equal to scoot's window
  gap and the two settings to change together, in
  [cli.md](../../cli.md#margins). The default look is unchanged: flush, square
  and opaque; making it floating stays a product call.
- **`scripts/scootbar-appearance-hw-test.sh`**, the hardware test, and its
  method in [testing.md](../../../../dev/research/scootbar-testing.md#the-appearance-hardware-test).
  Rehearsed on `--headless` and `--nested` (14 PASS, 0 FAIL, the pixel,
  zone, protocol and click checks).

**Resolved 2026-09-30, on the measured costs.** The "done when" is met: the
default look is good with no config (flush, square, opaque, unchanged), each
option is documented in [cli.md](../../cli.md), and the flush, rounded,
translucent and floating looks have measured costs on real `--tty` hardware,
published in the [resource ratchet](../lightest.md#appearance-looks-flush-against-floating):
14 PASS, 0 FAIL on the Asahi M2; the three shaped looks cost no more than the
flush one that a 10 ms-jiffy method can resolve, and all four idle at 0 jiffies.
That run had two outputs live (two bars), which the table says; one figure, the
translucent look's RSS, is a single run's difference to re-check, not a finding.
Making the run include a real clock needed one script change,
`SCOOTBAR_HW_FONT` (a NixOS box keeps its fonts in the store, where the script
did not look).

**What is not done, and where it went:** the `hover` token and per-module
token-per-state overrides (they need pointer motion, so
[pointer-and-interactions](pointer-and-interactions-done.md)), dot-style workspace
indicators, colors for inactive workspaces, a disc at any padding, and the
default margin matching scoot's gap (a product call) are filed together in
[appearance-followups](../appearance-followups.md). The opt-in animation question
is dropped, unmeasured (no frame callbacks were added). Per-output overrides
are [multi-output](multi-output-done.md).

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
    [multi-output](multi-output-done.md).
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
  [testing](../../../../dev/research/scootbar-testing.md#snapshots)): [testing-and-ci](testing-and-ci-done.md)
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
  [multi-output](multi-output-done.md).

## Coordination with scoot

The compositor's focus ring and rounded window corners are drawn by scoot; the
bar shares no code or config with them. Keep its default radius and colors
visually compatible, and document the tokens users would set to match.

## Done when

The default look is good with no config, each option is documented in the
same PR, and the flush and floating variants have measured costs published
with the [resource ratchet](../lightest.md).
