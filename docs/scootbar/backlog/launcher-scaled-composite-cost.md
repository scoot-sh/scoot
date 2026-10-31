---
title: "Layer::composite's scaled path costs 17 ns a pixel: a 2x launcher entrance frame is 9 ms"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# Layer::composite's scaled path costs 17 ns a pixel: a 2x launcher entrance frame is 9 ms

Filed 2026-10-30, found building #807 slice 4 (scootlaunch depth and
motion). Serves **daily-drive**: a HiDPI laptop is where the launcher's
scale-and-fade entrance is heaviest.

## The gap

`scootui::paint::Layer::composite` resamples bilinearly when the scale is
not within a 256th of one. Measured on the dev VM (release, `taskset -c 3`,
`dev/benches/shell-launch/`): a scaled entrance or fade frame of the
launcher is about 2.3 ms at 1x (414x314 surface) and 9 ms at 2x (828x628),
about 17 ns a destination pixel; the unscaled frames at either end of the
run cost 15 to 100 us. The budget at 60 Hz is 16.6 ms, so it fits, but it
is the heaviest thing the launcher does and the notification cards pay the
same per pixel.

Since 2026-10-31 ([launch-ignores-preferred-scale](resolved/launch-ignores-preferred-scale-done.md)) the launcher draws at the
output's scale, so the 2x case is no longer hypothetical: `bench_effects`
(release, `taskset -c 3`, three runs) gives an entrance frame of 8.85 to
8.98 ms median at 828x628, against 2.04 to 2.33 ms at 414x314.

## What to do

Make the per-pixel work cheaper without changing a byte of output (the
notifier's entrance goldens and the layer tests pin it): blend the two
source rows vertically once per destination row into a scratch row (the
weights are the same for every pixel of the row, and the integer arithmetic
is exactly the same as blending the columns first), then blend horizontally,
so the inner loop is a contiguous slice the compiler can vectorize. The
scratch row has to be pooled in the `Layer` (the composite allocates nothing
today, pinned by a test). Skipping rows that are wholly clear (a shadow's
room) is already done per pixel.

## Not in this ticket

Changing the entrance's look (the 96% start scale) or the curve.
