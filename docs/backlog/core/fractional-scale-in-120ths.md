---
title: "A scale that is not a multiple of 1/120 cannot be drawn exactly by any client"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# A scale that is not a multiple of 1/120 cannot be drawn exactly by any client

Found 2026-09-27 drawing scootbg at fractional scales
([hidpi-fractional-scale-done.md](../../scootbg/backlog/resolved/hidpi-fractional-scale-done.md)).

`wp_fractional_scale_v1.preferred_scale` is a count of 120ths, so a client
can only ever be told a multiple of 1/120. scoot takes any `[output] scale`
from 0.5 to 4 (`compositor/output_scale.rs`, `clamp_scale`) and renders at
exactly that, while Smithay's `set_preferred_scale` sends it rounded to the
nearest 120th. For 1.25 or 1.5 the two agree; for 1.33 scoot renders at
1.33 and tells clients 160/120 = 1.3333….

**Measured on scoot** (`--headless`, one 1600×1000 output, pixman),
scootbg drawing a one-pixel checkerboard the size of the output, unscaled
(`--mode center`), then scoot's own screenshot compared pixel by pixel:

| `[output] scale` | `preferred_scale` | surface (logical) | client buffer | pixels off the checker |
|---|---|---|---|---|
| 1.25 | 150 | 1280×800 | 1600×1000 | 0 of 1,600,000 |
| 1.5 | 180 | 1067×667 | 1601×1001 | 0 of 1,600,000 |
| 1.33 | 160 | 1203×752 | 1604×1003 | **1,596,598** of 1,600,000 |

At 1.33 the client's buffer (1203 × 4/3 = 1604) is correct by the protocol,
but scoot draws the surface 1203 × 1.33 = 1600 device pixels wide, so the
buffer is squeezed by four pixels across the output and every pixel is
resampled. No client can do better: the scale it would need is not one the
protocol can say.

**Who meets it:** anyone who sets a scale that is not a multiple of 1/120
(1.33, 1.1, 1.66, …), with any client that renders at the fractional
scale: text and images are resampled by a fraction of a pixel everywhere,
the blur fractional scaling exists to avoid. Scales people usually pick
(1.25, 1.5, 1.75, 2) are unaffected.

**Fix:** resolve `[output] scale` to the nearest multiple of 1/120 where it
is clamped (so 1.33 becomes 160/120), and use that one value everywhere:
rendering, `wl_output.scale`'s rounding up, `preferred_scale`, and the
`wlr-output-management` report. A config test pinning the rounding, and
this measurement as a compositor-side regression test (a client buffer of
`round(logical × v120 / 120)` lands one to one at 1.33). Say so in the
README's `[output] scale` entry.
