---
title: "scootlaunch --dmenu ignores the look's shadow, fill and motion"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# scootlaunch --dmenu ignores the look's shadow, fill and motion

Filed 2026-10-30, left out of #807 slice 4 on purpose. Serves **daily-drive**.

## The gap

`launch.toml`'s `shadow`, `fill`, `motion`, `motion-duration-ms` and
`motion-bounce` apply to the apps and run panels. `scootlaunch --dmenu` draws
with its own painter (`paint_dmenu`, fixed spacing, no stage) and never
builds the shadow room, the gradient or the entrance: a look that gives the
launcher a shadow gives a dmenu picker (the WiFi, Bluetooth and clipboard
pickers) none, so the two read as different apps. A pick prints and exits at
once, and a fade after the print would hold up a script that waits for EOF, so
the fade would have to come after the stdout is closed.

## What to do

Compose the dmenu panel on the same `Stage` (its painter takes a pixel slice
and a size, which is what the stage hands it), with the entrance and the
shadow but no exit fade: close stdout, then exit. `paint_dmenu` paints at
logical size straight into the device-sized buffer and takes no scale; check
what that draws on a 2x output before building on it.

## Not in this ticket

The sliding selection highlight in dmenu lists (the shared list widget draws
its own selection fill).

## Resolution

Landed 2026-10-31 (follow-up to #807). The dmenu panel composes on the same
stage as the apps panel: `paint_dmenu` paints into the panel-sized slice the
stage hands it, at a device scale (`dmenu_layout`), through the same chrome
as the apps panel so the `fill` gradient applies. Shadow, fill and the
entrance apply; there is no exit fade (`Pump::fade_out` returns at once for
dmenu) and no sliding highlight, so a pick or Escape prints and exits as
before and a script waiting on the pick never waits on motion.

The "check what that draws on a 2x output" question found a larger gap:
scootlaunch handles no scale event in any mode, so it draws 1x on every
output and the compositor scales it. Filed as
[launch-ignores-preferred-scale](launch-ignores-preferred-scale-done.md); the
dmenu painter is scale-correct and unit-tested at 1.5x, 2x and 3x so that fix
does not need to touch it.
