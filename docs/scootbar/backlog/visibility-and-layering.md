---
title: "Visibility and layering: top/bottom/overlay, exclusive zone or not, hide and show"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "skeleton-layer-surface"
---

# Visibility and layering

Filed 2026-09-29. Serves **daily-drive**.

## Facts to design around (`docs/protocols.md`, layer shell)

- Scoot draws all four layers; `background` and `bottom` sit behind windows,
  `top` and `overlay` in front.
- **A fullscreen window hides the `top` layer** (and it holds no keyboard
  there): the bar disappears while something is fullscreen, and the zone is
  covered. That is the desired default for a bar. An `overlay` bar would stay
  over fullscreen; offer it only as an explicit choice.
- An exclusive zone shrinks the tiling area; `-1` reserves nothing, which a
  floating overlay-style bar may want.
- A bar never asks for the keyboard (`keyboard_interactivity: none`), so
  clicking it never moves window focus.

## What to build

- `layer = "top" | "bottom" | "overlay"`, `edge = "top" | "bottom"`, and
  `exclusive = true | false` (zone or float over windows).
- `scootbar msg hide | show | toggle`: **hiding destroys the surface and its
  buffers**, so a hidden bar costs nothing but the process; showing rebuilds
  it. The exclusive zone is released on hide so windows reclaim the space.
- Auto-hide with a reveal on pointer contact needs a thin always-present
  sensing surface; measure its cost and decide if it is worth having at all.
- Vertical bars (left/right) are **not in v1**: layout is horizontal. Record it
  as a deliberate omission so it is not lost.
- A bar bound to a keybinding: document `scoot`'s bind running
  `scootbar msg toggle` as the way, rather than a bar-side hotkey (the bar
  takes no keyboard).

## Edge cases

Hide during a redraw, show while an output is missing, toggling faster than
the compositor answers (coalesce), a `top` bar during a fullscreen window
(nothing to draw, nothing to do).

## Done when

Each layer and edge combination is verified on headless scoot, a hidden bar
holds no shm buffer, and toggling round-trips without windows jumping.
