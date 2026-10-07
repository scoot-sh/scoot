---
title: Transitions between wallpapers
description: "Animate a wallpaper change on scootbg — fade, wipe, grow, duration, easing, and the [wallpaper] keys."
---

Animate the change from one wallpaper to the next, instead of landing
at once: a fade, a wipe, or a grow, with a duration and an easing
curve. It costs at most two extra buffers while it runs (one, where
the compositor releases promptly) and nothing after.

```sh
scootbg set '#101014' --transition fade --duration-ms 800
scootbg set ~/Pictures/city.png --transition wipe --angle 90
scootbg set ~/Pictures/grid.png --transition grow --position 0,0 --output DP-1
```

![Four frames of a wipe from red to blue, left to right](../../../assets/scootbg-transition-strip.png)

While a transition runs, `query` reports it: `"transition":"fade"`
(`"wipe"`, `"grow"`, else `null`). `set` returns once the last frame
is on screen, so a screenshot taken straight after shows the target —
and a newer `set` mid-transition starts from the frame showing then,
never queued behind the old one. How each kind paces and damages its
frames is [below](#how-it-works); the raw protocol is in
[README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#the-control-protocol).

## Options

Each flag has a matching `[wallpaper]` key, so scoot animates its own
changes the same way. Every key reloads live (re-applied on
`scoot msg reload` when the section changed).

| Flag / key | Type | Default | Example | Reloads live |
|---|---|---|---|---|
| `--transition` / `transition` | `none`, `fade`, `wipe`, `grow` | `none` | `--transition fade` | yes |
| `--duration-ms` / `duration-ms` | integer ms, `0`–`60000` | `500` | `--duration-ms 800` | yes |
| `--easing` / `easing` | `linear`, `ease-in`, `ease-out`, `ease-in-out`, `smooth` | `ease-out` | `--easing linear` | yes |
| `--angle` / `angle` | degrees as a number | `0` | `--angle 90` | yes |
| `--position` / `position` | `X,Y` fractions, each `0`–`1` | `0.5,0.5` | `--position 0,0` | yes |

A transition flag without `--transition` is refused (exit 2), so a stray
`--duration-ms` teaches rather than being ignored; with an explicit
`--transition none` the rest are ignored. Past `60000` ms is refused:
a longer animation would hold its buffers past any reasonable change.
`clear` takes no transition and lands at once — there is no wallpaper
to blend from or to. In TOML the keys are strings:

```toml
[wallpaper]
color = "#1e1e2e"
transition = "fade"
duration-ms = "800"

[wallpaper.output."DP-2"]
image = "/srv/city.png"
transition = "wipe"
angle = "90"
```

Each table stands alone, as for the wallpaper itself: an output's
change does not take the top level's transition, and an output with no
table follows the top level. A section an older scootbg does not know
these keys in is refused loudly (exit 2, the session carries on), while
a `set` from a newer client on an older daemon shows the wallpaper
without the animation.

## The kinds

**Fade** lerps every pixel from old to new — the simplest kind, and the
most expensive per frame (about 5.4 ms at 1080p, 21.6 ms at 4K,
measured; at 4K it holds time with fewer steps, see
[below](#how-it-works)):

```sh
scootbg set '#101014' --transition fade --duration-ms 800 --easing smooth
```

**Wipe** sweeps a straight edge across at `--angle` degrees, clockwise
from the positive x-axis: `0` brings the new wallpaper in from the left
edge, `90` from the top, `180` from the right, `270` from the bottom.
Behind the edge is new, ahead of it old:

```sh
scootbg set ~/Pictures/city.png --transition wipe --angle 90 --duration-ms 600
```

**Grow** grows a disc of the new wallpaper from `--position` (fractions
of the width and height; `0.5,0.5` is the center) until it covers the
output:

```sh
scootbg set ~/Pictures/grid.png --transition grow --position 0,0
```

At `t = 0` every pixel is old and at `t = 1` every pixel is new,
exactly — no stray line of either side at either end.

## How it works

CPU only, into `wl_shm`, one frame at a time: each frame blends the two
endpoint buffers (shared references to pixels that already exist, never
copies) into a full-size frame buffer — the third buffer the ticket
budgets, 33 MB at 4K, with a second where the compositor releases
lazily — and commits it with damage limited to what changed (the
whole buffer for a fade, the sweeping band for a wipe, the disc's box
for a grow). Frames are paced by frame callbacks, with presentation
feedback where the compositor offers `wp_presentation` (a discarded
frame just jumps the clock ahead) and a 60 Hz timer covering the rest;
the eased progress always comes from the clock, so a slow frame drops
frames rather than falling behind, and a frame past its 8 ms budget
skips the next one. At 1080p every kind holds 60 fps outright (2.4–5.4
ms a frame, measured); at 4K a fade blends in 21.6 ms, a wipe in
12.1 ms, a grow in 10.0 ms, so 4K transitions keep time with fewer
steps instead of lagging. A restart snapshots what is on screen with one
memcpy and starts from there. When the last frame lands, the final
wallpaper goes on through the normal draw and every extra buffer is
freed: an idle daemon costs what a static wallpaper costs (zero
wakeups, measured). Allocation failure ends the transition at once at
the final wallpaper — never wedged.

> **Symptom:** *the change lands at once, no animation.*
> No `--transition` on the request (the default is `none`), a zero
> duration, or a `clear` (always instant). With a `[wallpaper]` section,
> check the keys are strings and the table you mean carries them.
>
> **Symptom:** *`set` says `unknown transition` / `bad duration-ms` / ...*
> The value is not one listed above. Durations are digits in
> milliseconds (`0.5` is not one); angles are plain numbers (`90deg` is
> not one); positions are two fractions with a comma (`0.5, 0.5` with a
> space is not one).
>
> **Symptom:** *an older scootbg refuses the section.*
> Transition keys need the scootbg that knows them: update scootbg (it
> ships with scoot), or drop the keys. A newer client's `set` on an
> older daemon instead shows the wallpaper without animating.

## See also

- [The `[wallpaper]` section](./index.md#the-wallpaper-section) — the
  keys above in scoot's config, and what wins when both change.
- [CLI reference](./cli.md) — when `set` returns with a transition,
  and the `query` output mid-flight.
- [Troubleshooting](./troubleshooting.md) — transitions refused or
  landing at once.
