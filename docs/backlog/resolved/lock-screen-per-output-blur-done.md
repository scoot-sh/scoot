---
title: "Lock blur follows the live per-output wallpaper"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-06"
---

# Lock blur follows the live per-output wallpaper

RESOLVED 2026-11-06. scootbg now keeps a blurred picture of what each
output shows, and the locker passes them to swaylock. The user-facing
reference is `site/src/content/docs/scootbg/index.md` ("A picture for your
lock screen") and `site/src/content/docs/desktop/idle-lock.md`.

What landed, and what the ticket left open:

- **The blur is of the rendered output buffer, not the decoded image.**
  It is what the output shows after fit, crop and mode, so the locker's
  `fill` keeps the wallpaper's framing. It is made on the decode worker
  thread, right after the buffer is drawn (`lockblur::blurred_png`): area
  average down to at most 320 px on the long side (reading a few rows of
  each box, not all), three box blurs at 3.1 percent of the width (the
  build-time blur's strength), a fast-deflate PNG. About 10 ms at 4K on
  the dev VM and 50 KB for a photograph; the whole `set` on a 4K output
  went from a median of 203 ms to 208 ms (three runs of twelve each,
  release builds, on the dev VM). The small PNG is kept with the pixels
  (`canvas::Memory::lock`), so a workspace switch re-points the lock
  screen with no work.
- **The interface is a directory, not a request.** `$XDG_RUNTIME_DIR/
  scootbg-NAME.lockscreen/` holds `output-N.png` files (N the daemon's own
  output number: an output name never reaches a path) and an `index` of
  `name<TAB>file` lines, each file and the index replaced atomically. No
  control-protocol change and no `scootbg` binary needed by the locker:
  `scoot_lockscreen_images` in `nix/modules/lock-images.sh` reads it in
  bash. Names a locker could not take whole (a `:`, a control byte,
  nothing) get no line.
- **What follows what.** A picture follows the output's choice
  (`Choices::for_output`, so the active workspace), not the frames of a
  transition. A `clear` removes the picture; a color wallpaper gets a flat
  picture of the color; an unplugged output loses its line and file.
- **Fallback.** swaylock shows an output's own `-i OUTPUT:FILE` image
  over the config's global `image` and skips one that fails to load, so
  the chain is live picture, then the built blur, then `color`, with no
  logic of our own. The live pictures are off when the lock is not themed
  and when `lock.settings` sets its own `image`.
- **Lifetime.** A start empties the directory (after the socket claim, so
  a second daemon cannot wipe a live one's), a clean exit removes it, a
  signal leaves the last pictures.

Evidence is in the PR.

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
