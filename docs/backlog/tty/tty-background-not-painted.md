---
title: "Under `--tty`, the background color is not painted where no window covers it \u2014 the uncovered area stays black."
status: "open"
area: "tty"
priority: "medium"
blocked: null
---

# Under `--tty`, the background color is not painted where no window covers it — the uncovered area stays black.

Under `--tty`, the background color is not painted where no window
covers it — the uncovered area stays black. Found while bug-bashing
item 17, 2026-09-13; *not* caused by it. `MODE=--tty
scripts/smoke-test.sh` fails one assertion: `the background pixel at
(3,3) is rgb(0,0,0), expected #123456 (rgb(18,52,86))`. The same
script's default `--headless` run passes all 12 assertions, and the
`--tty` run's focus-ring pixel checks at (403,9) and (1197,9) both pass
— so windows, decorations and the screenshot path are all fine; it is
specifically the area no client covers. Reproduced identically against a
binary built from `main` at `868dd83` (`diff` of the two runs'
assertion lines is empty), so it predates item 17 and is a real `--tty`
bug rather than a smoke-test artifact. No diagnosis done: the obvious
place to look first is the damage/buffer-age path, which `--tty` engages
(`Tty::next_buffer_age`/`advance_generation`) and `--headless` does not,
so a never-damaged region on the first frame would keep whatever the
render target was initialized to.
