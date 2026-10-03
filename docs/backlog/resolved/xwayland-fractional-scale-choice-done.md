---
title: "XWayland: let the user choose sharp or light X apps at a fractional scale (default sharp)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# XWayland: sharp or light X apps at a fractional scale

Filed 2026-09-28 with the user, from
[scale-aware X windows](../resolved/xwayland-scale-aware-done.md). Serves
**daily-drive** on memory-tight, GPU-less machines running X apps at a
fractional `[output] scale`.

## Today

At a fractional scale (1.25, 1.5, ...) X toolkits can only draw at an
integer, so scoot has X draw at `ceil(scale)` and downscales: sharp, but
each X window's buffers are 4x what they were when X drew at 1 and scoot
upscaled. Measured per-frame composite cost is the same either way; the
cost is memory (`docs/configuration.md`, `[output] scale`):

| Output and scale | X window | X at 1 (light) | X at 2 (sharp, today) |
|---|---|---|---|
| 1920x1080 at 1.25 | full screen, 1536x864 logical | ≈10.6 MB held | ≈42 MB held |
| 3840x2160 at 1.5 | full screen, 2560x1440 logical | ≈29 MB held | ≈118 MB held |
| any, at 1.25/1.5 | an 800x600-logical dialog | ≈3.8 MB held | ≈15 MB held |

("Held" is the buffer twice: the X server's pixmap and the shared-memory
buffer.) Integer scales and scale 1 are unaffected by the choice.

## What to do

An option, e.g. `[xwayland] fractional = "sharp" | "light"`, **default
`"sharp"`** (today's behaviour, decided with the user 2026-09-28):

- `"sharp"`: X draws at `ceil(scale)` (bounded by the layout, as today --
  `xwayland/scale.rs`'s `fit_x_scale`).
- `"light"`: at a non-integer scale X draws at `floor(scale)` (1 below 2),
  and scoot upscales -- blurry, about a quarter of the memory.

Route it through the one chooser (`fit_x_scale` / `refit_xwayland`) so a
reload of the option re-applies live like a scale change (client scale,
XSETTINGS, configures re-clamped). Scale 1 and integer scales stay
byte-for-byte unchanged either way; pin that. Tests: the chosen X scale per
option at 1.25/1.5/2, a live reload between the two, and the memory claim
(an X window's buffer size) per option.

## Resolution (2026-10-03, PR #398)

Landed as `feat(scoot): [xwayland] fractional sharp-or-light choice at fractional scales` (`ac9c30cd`). Per-output ceiling (`ceil` sharp / `floor` light, largest wins) feeds the unchanged `fit_x_scale` bound; reload re-applies live via `refit_xwayland`. Review found no blocking issues (4 low: 4 sharp-only doc statements qualified in the follow-up commit, transient double-refit on combined reloads, mixed-scale test gap, trim cosmetic). CI green including the live-XWayland job. Document it in
`docs/configuration.md` `[xwayland]` and the `[output] scale` row.
