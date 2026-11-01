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

A 3x output (about 1240x940) extrapolates to roughly 20 ms an entrance
frame at the same 17 ns, over the 60 Hz budget; unmeasured, filed from the
#839 review as [launch-fit-remainder-and-scale-len](resolved/launch-fit-remainder-and-scale-len-done.md).

Measured 2026-11-01 on the dev VM (`bench_effects`, release, `taskset -c 3`,
three runs, 2,700 frames each): the entrance frame at 3x (1242x942
surface) costs 19.3 to 19.5 ms median (p5 3.9 to 4.0 ms; one unpinned run:
19.9 ms median, p5 4.1 ms) — over the 16.6 ms frame budget, as
extrapolated. The p5 tail is the run's end frames, where the scale is
within a 256th of one and the copy is exact. No bigger optimization is
attempted here: the cheapest fix stays the one filed below (blend rows
vertically into a pooled scratch row, then horizontally, so the inner
loop vectorizes, with no output change).

The notification cards pay the same
([notify-ignores-preferred-scale](resolved/notify-ignores-preferred-scale-done.md)):
`bench_motion --scale N` (release, `taskset -c 3`, dev VM, a stack of five
cards, the median of 400 frames per run, six runs per scale) gives an
`enter scale` frame of 0.6 to 1.3 ms at 1x (384x522), 2.5 to 3.0 ms at 1.5x,
1.3 to 5.0 ms at 2x and 2.8 to 11.3 ms at 3x (1152x1566): under 16.6 ms at
every scale, the 3x case with least room. (The VM's CPU runs in two speeds,
so runs of the same binary differ by up to 3x; the 3x worst case is the one
to read.) The slide frames, which composite unscaled, are 0.4 to 0.7 ms at
1x and 1.6 to 3.1 ms at 2x.

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
