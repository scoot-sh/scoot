---
title: "scootlaunch --dmenu ignores the look's shadow, fill and motion"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
