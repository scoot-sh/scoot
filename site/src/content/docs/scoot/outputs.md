---
title: Outputs
description: "Scales, per-output modes, moving across monitors, and hotplug."
---

One scale for everywhere, or one per monitor. `[output]` holds the
default; `[[outputs]]` entries override per connector (matched by the
name `scoot msg outputs` lists: `eDP-1`, `DP-1` under `--tty`;
`headless`, `headless-2` under `--headless`).

```toml
[output]
scale = 1.5            # every output without an entry of its own

[[outputs]]
name = "eDP-1"         # the laptop panel
scale = 2.0

[[outputs]]
name = "DP-1"          # an external monitor
scale = 1.0
mode = "1920x1080"
position = [-1920, 0]  # left of the primary
```

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[output] scale` | float | `1.0` | live | Output scale advertised to clients and rendered at. Clamped into `0.5..=4.0` with a warning, then resolved to the nearest 1/120 (so `1.33` becomes `160/120`). `--nested` ignores a non-1.0 value with a warning — the host owns the scale. |
| `[[outputs]] name` | string | required | — | The output this entry is for, matched exactly (case included) against its connector name. An entry for a monitor that is not connected waits for it and applies when it is plugged in. |
| `[[outputs]] scale` | float | `[output] scale` | live | This output's scale, resolved like `[output] scale`. |
| `[[outputs]] mode` | `"WxH"` | `--mode` / `--width`+`--height` | live | Under `--tty`, which connector mode to drive (falls back to the preferred mode with a warning); under `--headless`, that output's size. A reload resizes the output like a hotplug would; a mode no connector offers keeps the running size, and the reload reply says so (`outputs.DP-1.mode (could not switch to 1920x1080; kept 1600x900)`) instead of reporting it applied. |
| `[[outputs]] position` | `[x, y]` | packed | live | This output's origin in logical pixels — either axis may be negative, so `[-1920, 0]` puts a 1080p monitor left of the primary. An output without one packs left to right in creation order from the origin, stepping over placed outputs. Overlaps and gaps are allowed (see below). |

A scale can also change without touching the file:
`scoot msg output-scale DP-1 1.5` sets one live (by connector name or
id), and `scoot msg output-scale DP-1 reset` drops it again. That is
runtime state, like `output-power`: it beats the file's scale for that
connector (a replugged monitor comes back at it), and a successful
reload (which lists each scale it moved in its `applied`) or a restart
goes back to the file's scales. It takes the same
range, refusing a value outside it instead of clamping. The
[display profiles](../desktop/index.md#displays) watcher drives scale
this way. See [requests](../msg/requests.md).

A window is told the scale of the output it is placed on, and re-told
when it moves; popups, subsurfaces, bars and lock surfaces follow their
own output. At a fractional scale X apps cost memory (see
[XWayland](./xwayland.md)): each X window's buffers quadruple against
drawing at 1.

## Moving across outputs

Three pairs of actions reach across screens — stepping relatively, and
naming a screen by position or by id. The relative pair has the default
binds:

```toml
[binds]
"super+comma" = "focus-output-left"
"super+period" = "focus-output-right"
"super+shift+comma" = "move-window-to-output-left"
"super+shift+period" = "move-window-to-output-right"
```

Each steps to the neighbouring output in geometry order, wrapping
around — with two monitors either key names the other. Positions
(`focus-output-index N`, 0-based in creation order) and ids
(`focus-output ID`, from `scoot msg outputs`) name fixed screens
instead. The stability rule: output ids are never reused, so a monitor
that is unplugged and plugged back in comes back under a *new* id —
the default stepping binds keep reaching every monitor across a
replug, while an explicit id bind names the output that went away.

A returning monitor gets its windows back: when an output is removed
its workspaces are adopted by the remaining output, and when a monitor
with a matching identity returns, the still-open ones move back (a
window moved elsewhere by hand stays where it was put). Adopted
workspaces announce themselves to bars with their monitor's name
(`"2 DP-1"`, back to `"2"` on restore).

> **Symptom:** after a replug, a bind reaches the wrong (or no) screen.
> `scoot msg outputs` lists the fresh ids — rebind the new one, or stick
> to the stepping binds, which never name an id at all.

## Arranging monitors

Without a `position`, outputs pack left to right in creation order from
the origin — the second monitor lands immediately right of the first, the
third right of the second. A `position` pins one monitor's origin in
logical pixels (what `scoot msg outputs` shows as `x` and `y`), so a
monitor standing left of the laptop goes at `[-1920, 0]` and one stacked
above it at `[0, -1080]`. Outputs without a `position` pack past the
placed ones: a placed monitor never shoves an earlier output aside, so
the primary stays at the origin and a replugged monitor lands back on its
entry while its neighbours stay put.

Three consequences to know before spreading monitors around:

- **Overlaps are allowed.** Two outputs on the same origin each render
  their own strip; the left/right stepping binds still ring them in
  `x`-then-`y` order. scoot will not refuse or nudge them apart.
- **Gaps are allowed too.** The pointer roams the bounding box of every
  output, so it can rest in a gap where no output is — exactly as it
  already can over uneven heights. Windows never strand there: each one
  sits on exactly one output.
- **A mistyped `position` costs only the position.** `position = "left"`
  or `position = [0]` warns naming the entry and packs that output,
  instead of failing the file. An unknown key inside the entry still
  fails the whole file, like every other table.

> **Symptom:** the pointer vanishes between two monitors, then reappears
> on the next one. It is crossing a gap: the pointer roams the bounding
> box of every output, and no screen draws the cursor where no output is.
> `scoot msg outputs` shows the rectangles — placing the outputs adjacent
> removes the dead zone.
