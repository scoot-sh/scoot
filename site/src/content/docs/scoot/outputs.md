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
```

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[output] scale` | float | `1.0` | live | Output scale advertised to clients and rendered at. Clamped into `0.5..=4.0` with a warning, then resolved to the nearest 1/120 (so `1.33` becomes `160/120`). `--nested` ignores a non-1.0 value with a warning — the host owns the scale. |
| `[[outputs]] name` | string | required | — | The output this entry is for, matched exactly (case included) against its connector name. An entry for a monitor that is not connected waits for it and applies when it is plugged in. |
| `[[outputs]] scale` | float | `[output] scale` | live | This output's scale, resolved like `[output] scale`. |
| `[[outputs]] mode` | `"WxH"` | `--mode` / `--width`+`--height` | restart only | Under `--tty`, which connector mode to drive (falls back to the preferred mode with a warning); under `--headless`, that output's size. A reload refuses a changed `mode` by name. |

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
