---
title: "Visibility and layering: top/bottom/overlay, exclusive zone or not, hide and show"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-09-30"
---

# Visibility and layering

## Resolution (2026-09-30)

Landed: `[bar] layer` / `--layer` (`bottom`, `top`, `overlay`), `exclusive` /
`--exclusive` (`false` sends zone -1), and `scootbar msg hide|show|toggle`
(hiding destroys every layer surface and buffer, releases the zone, and is
applied once per loop turn so a burst of toggles is one change; `{"type":"bar","visible":B}`
is the reply). Reference: [cli.md](../../cli.md#layers-and-the-zone).
Verified on headless scoot in `crates/scootbar/tests/visibility.rs`: every layer
on both edges with and without the zone, the layer and zone on the wire, a
hidden bar holding zero shm mappings, forty concurrent toggles settling on the
net result, a reload keeping a hidden bar hidden, and a window returning to its
rectangle with no intermediate one.

Decided, not built: **vertical bars** are a deliberate omission (the layout,
modules and hit-test are horizontal); **auto-hide** is left out, decided from
the design rather than measured (a sensing surface is what `hide` removes; a
`scoot` bind running `scootbar msg toggle` costs nothing unused), to be
revisited with `pointer-and-interactions`.

Filed 2026-09-29. Serves **daily-drive**.

## Facts to design around (`docs/protocols.md`, layer shell)

- Scoot draws all four layers; `background` and `bottom` sit behind windows,
  `top` and `overlay` in front.
- **A fullscreen window hides the `top` layer** (and it holds no keyboard
  there): the bar disappears while something is fullscreen, and the zone is
  covered. That is the desired default for a bar. An `overlay` bar would stay
  over fullscreen; offer it only as an explicit choice.
- **Fullscreen and maximize are different on purpose.** Fullscreen hides the
  bar; a window that should fill the screen *with* the bar visible wants
  scoot's [maximize](../../backlog/core/maximize.md), which does not exist yet.
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
- Vertical bars (left/right) are **not in the first version**: layout is horizontal. Record it
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
