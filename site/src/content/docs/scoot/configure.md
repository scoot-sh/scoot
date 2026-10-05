---
title: Configure
description: "The config file, live reload, autostart, and what happens when the file is wrong."
---

Make scoot yours. One TOML file holds everything — layout, appearance,
outputs, binds, floating rules, autostart, wallpaper — and most of it
re-applies live with `scootctl reload`. No config file yet?
`--print-default-config` writes you a starting one (below).

## The config file

`--config PATH` loads a TOML file explicitly. Without it, scoot looks
for `$XDG_CONFIG_HOME/scoot/config.toml`, falling back to
`~/.config/scoot/config.toml`, and runs on built-in defaults if
neither exists. Ten optional tables: `[layout]`, `[appearance]`,
`[output]`, `[renderer]`, `[tty]`, `[xwayland]`, `[autostart]`,
`[floating]`, `[wallpaper]`, `[binds]` — plus any number of
`[[window_rule]]` entries. Every field in every table is itself
optional and defaults independently, so a config that only sets `gap`
leaves everything else at its built-in default.

```sh
scoot --print-default-config > ~/.config/scoot/config.toml
```

writes a starting file to stdout (never to a path, so it cannot clobber
anything), generated from the same defaults this section documents —
every key present and commented out with its default as the value, so
the file as-is is exactly the defaults. `--write` places that same
emission at the default location directly, private to you (`0o600`),
and refuses loudly rather than overwriting anything already there.
(On a machine with no `scoot` binary — macOS, where only the `scootctl`
client builds — copy the [example below](#example-configtoml) instead.)

### Failure semantics

An explicit `--config PATH` that doesn't exist or can't be read is a
hard startup error — you pointed at it on purpose. Every other problem
falls back to defaults and logs instead of blocking startup, with two
exceptions (both cases where guessing would be worse than refusing):

- No file at the *default* path: silent, not even a log line (a fresh
  install, not a mistake).
- Malformed TOML, an unknown field, or a wrong-typed field **anywhere
  in the file**: logged as an error, and the *entire* file is discarded
  for full built-in defaults — a single bad field in `[layout]` also
  throws away an otherwise-valid `[binds]` table.
- One bad `[appearance]` color, one bad `[binds]` entry, one bad
  `[autostart]` entry, or one unusable `[[window_rule]]`: logged as a
  warning, and only that field/bind/entry/rule falls back — everything
  else still applies.
- Anything wrong inside `[wallpaper]`: logged as an error, and only the
  wallpaper is skipped — the rest of the file applies.
- A set-but-unusable `[tty] gpu`: a hard startup error naming the key,
  not a silent fallback to the automatic pick. The same for
  `[renderer] backend = "gles"` with no working EGL.

The reason for the general rule: on `--tty` scoot *is* the session —
there is no other compositor to fall back to, so it always starts with
something usable and says what's wrong in the log instead.

## Reloading the config

Two triggers re-read the same file startup used and re-apply what can
be re-applied live:

- `scootctl reload` (and `scoot msg reload`), which answers with an
  applied-vs-refused report;
- `kill -HUP <compositor pid>`, which drives the same path with no
  reply channel (the summary goes to the compositor log instead).

No file watching: a live-edited config would fire mid-keystroke, while
both triggers above say exactly when.

**Applied live:** layout (gap, column widths, default width), output
scales, appearance (ring, background, corners, cursor), the whole
`[binds]` table, new `[autostart]` spawn entries, `[floating]` and the
`[[window_rule]]`s (for windows that map after the reload), `[wallpaper]`
(handed to scootbg again), `[xwayland] fractional`. The reply names
each applied field:

```json
{ "type": "reloaded", "applied": ["layout.gap", "binds"],
  "refused": ["tty.gpu (takes effect on restart: the session already drives its device)"] }
```

**Refused, explicitly, pending a restart:** `[tty] gpu`, `[renderer]
backend`, `[xwayland] enabled`, and an `[[outputs]]` entry's `mode` —
all four take effect on restart, and each refusal says so. A reload
that cannot load the file at all answers an `error` instead, keeps the
running config untouched, and logs — never defaults, never a
half-applied session, never an exit.

Two guarantees the applied set pins: a key held across a `[binds]`
rebuild neither wedges nor drops; and under `--tty` a reload cannot
strip the `Ctrl+Alt+F1..F12` VT-switch recovery bindings — they are
layered back on last, overriding any colliding file bind with a
warning. A reload applies while the session is locked, except new
autostart entries (a spawned program at lock time could disclose onto
the locked session, so those wait for the first unlocked reload).

> **Symptom:** a reload changed nothing and both lists are empty.
> That means "the reload changed nothing it was asked to" — the file
> and the session already agree. Edit first, then reload.

## Autostart

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `commands` | array of strings | `[]` | new `spawn` entries run once each | Action strings to run once each, in file order, at session startup — before the `--` command. |

Each entry is an action string — something `scoot msg action` would
accept and a `[binds]` value could contain, which is what keeps the
config agent-legible:

```toml
[autostart]
commands = [
    "spawn waybar",
    "spawn mako",
]
```

Fail-open per entry: a malformed entry is skipped with a warning
naming just that entry; every other entry still runs, and the session
always starts. No supervision: an entry that exits instantly is
reaped, not restarted. A reload runs the spawn delta (entries the
session has not seen yet run once each; a reloaded non-`spawn` —
`quit` included — is refused by name, so a reloaded `quit` cannot end
the session).

### Idle: locking and screen power

(The manual recipe for sessions outside the flake.) scoot provides the protocols; the policy ships with [the desktop
profile](../desktop/index.md#idle-and-lock). Outside the flake, the
same policy is a hand-written swayidle setup — `spawn` splits on
whitespace with no shell, so a swayidle line lives in a small script
the session starts:

```sh
#!/bin/sh
# ~/.config/scoot/idle.sh
exec swayidle timeout 600 'swaylock' timeout 900 'wlopm --off \*' resume 'wlopm --on \*'
```

(The `\*` is load-bearing: swayidle runs each command through `sh -c`,
which would glob a bare `*` against its working directory — the
backslash reaches `sh` intact inside the single quotes and leaves
`wlopm` a literal `*`, its "every output".)

```toml
[autostart]
commands = [
    "spawn /home/you/.config/scoot/idle.sh",
]
```

Lock after ten minutes, panels off after fifteen, back on at the first
input. The same state is drivable over IPC for agents and scripts
(`scootctl output-power 1 off`, `scootctl outputs` reporting it).

## Example config.toml

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]
default_column_width = 1

[appearance]
focus_ring_width = 4
focus_ring_inactive_width = 2
focus_ring_active_color = "#ffaa00"
focus_ring_inactive_color = "#333333"
background_color = "#101014"
cursor_size = 24
cursor_color = "#ffcc66"
# Unset follows $XCURSOR_THEME, then "default" -- i.e. the rest of the
# desktop. Name one here only to override that.
# cursor_theme = "Adwaita"
prefer_no_csd = true

[output]
# 1.0 is correct for a non-HiDPI display; raise it (e.g. 2.0) on a HiDPI
# panel, or text and widgets render far too small.
scale = 1.0

# A HiDPI laptop panel beside an ordinary monitor: each its own scale, by
# the connector name `scootctl outputs` lists.
# [[outputs]]
# name = "eDP-1"
# scale = 2.0
#
# [[outputs]]
# name = "DP-1"
# mode = "1920x1080"

# [renderer]
# Unset means "pixman", the CPU renderer -- the right answer on a GPU-less
# box and the default everywhere. "gles" is opt-in; under --tty it scans out
# from the GPU in a gpu-scanout build. --renderer wins over this when both
# name one.
# backend = "gles"

# [tty]
# Uncomment only on hardware where the automatic DRM device search picks
# wrong. Unset means the automatic search picks; --gpu PATH on the command
# line wins over this when both name one. Name the display controller,
# never the render node, and prefer a stable /dev/dri/by-path/... alias.
# gpu = "/dev/dri/by-path/platform-soc:display-subsystem-card"

[binds]
"super+n" = "focus-column right"
"super+shift+n" = "move-column right"
"super+t" = "spawn foot"
"super+shift+t" = "spawn foot -e htop"
"ctrl+alt+space" = "spawn fuzzel"

[autostart]
commands = [
    "spawn waybar",
    "spawn mako",
]

[floating]
# Dialogs, transient and fixed-size windows float when they map; false
# tiles everything except what a rule floats.
auto = true
# Alt+drag moves and resizes floating windows (Super is the default).
modifier = "alt"

# pavucontrol's app id is org.pulseaudio.pavucontrol (`scootctl windows`
# shows any window's).
[[window_rule]]
match_app_id = "*pavucontrol"
float = true
size = [700, 500]

# Needs scootbg installed (the Nix modules do it for you).
[wallpaper]
image = "~/Pictures/hills.jpg"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```

Next: [Layout](./layout.md) and [Appearance](./appearance.md) — every field of the two tables you will touch most.
