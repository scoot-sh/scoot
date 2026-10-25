---
title: "Lock blur follows the live per-output wallpaper"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Lock blur follows the live per-output wallpaper

Filed 2026-10-24. Serves **daily-drive** (the lock shows what the
session shows: the wallpaper actually on each screen, not the one the
last rebuild knew).

## The gap

Since the blurred-lock change the locker shows a blur baked at build
time from the look's shipped wallpaper (`programs.scoot.desktop.idle`,
`image=` pointing at a store PNG). That is correct for every shipped
look, but scootbg's wallpaper is live state: `scootbg set IMAGE
--output DP-1` (and per-workspace mappings) change what is on screen
without a rebuild, and a look applied live (`scoot look apply`,
`scoot look next`) changes it too. Until the next rebuild the lock
keeps showing the built blur -- still blurred, themed and messaged,
but not the current image. A live screenshot is not the fix: this
compositor deliberately publishes no `wlr-screencopy` (it speaks
`ext-image-copy-capture` instead, which stock lockers do not read), so
there is nothing for a locker to capture against scoot.

## What to do

Have scootbg (which owns the current image per output) maintain a
blurred copy per output, regenerated on every wallpaper change (CPU
box blur over the decoded buffer, cached, never per frame), and teach
the locker script to pass one `-i <output>:<blurred>` per live output
(output names enumerated at lock time, falling back to the built blur
and then to `color` -- a lock that cannot theme must still lock). Pin:
a `scootbg set` on one output changes only that output's lock image;
unplugging the output drops its image; the fallback chain still locks
with the plain color when no image exists.

## Not in this ticket

The build-time blur itself (shipped with the blurred lock), and any
`wlr-screencopy` support in the compositor (deliberately not
published).
