---
title: Configure
description: "The config file, live reload, autostart, and what happens when the file is wrong."
---

One TOML file holds everything — layout, appearance,
outputs, binds, global shortcuts, floating rules, autostart, wallpaper — and most of it
re-applies live with `scoot msg reload`. No config file yet?
`--print-default-config` writes a starting one (below).

## The config file

`--config PATH` loads a TOML file explicitly. Without it, scoot looks
for `$XDG_CONFIG_HOME/scoot/config.toml`, falling back to
`~/.config/scoot/config.toml`, and runs on built-in defaults if
neither exists. Thirteen optional tables: `[layout]`, `[appearance]`,
`[output]`, `[[outputs]]`, `[renderer]`, `[tty]`, `[xwayland]`, `[virtual_input]`,
`[autostart]`, `[floating]`, `[wallpaper]`, `[binds]`, `[global_shortcuts]` — plus any number of
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
(On a machine with no `scoot` binary — an older install without it —
copy the [example below](#example-configtoml) instead.)

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
  `[renderer] backend = "gpu"` with no working EGL.

The reason for the general rule: on `--tty` scoot *is* the session —
there is no other compositor to fall back to, so it always starts with
something usable and says what's wrong in the log instead.

## Reloading the config

Two triggers re-read the startup file and re-apply what can go live:

- `scoot msg reload` (and `scoot msg reload`), which answers with an
  applied-vs-refused report;
- `kill -HUP <compositor pid>`, which drives the same path with no
  reply channel (the summary goes to the compositor log instead).

No file watching: a live-edited config would fire mid-keystroke, while
both triggers above say exactly when.

**Applied live:** layout (gap, column widths, default width), output
scales and positions, `[[outputs]]` modes (a live resize per backend —
`--nested` refuses them, the host owns the size), appearance (ring, background, corners, cursor), the whole
`[binds]` table, the `[global_shortcuts]` chords, new `[autostart]` spawn entries, `[floating]` and the
`[[window_rule]]`s (for windows that map after the reload), `[wallpaper]`
(handed to scootbg again), `[xwayland] fractional`. The reply names
each applied field:

```json
{ "type": "reloaded", "applied": ["layout.gap", "binds"],
  "refused": ["tty.gpu (takes effect on restart: the session already drives its device)"],
  "shadowed": ["appearance.background_color"], "invalid": [] }
```

`shadowed` lists every file key that overrides the active look (see
[Appearance](./appearance.md#the-look-layer)); `invalid` lists file keys
whose bad values fell back to the look's. `scope` names which reload ran:
absent for a plain reload, `"look"` for a look-scoped one (below). The
reply answers once the wallpaper run is handed over, without waiting for
it. The look itself
lives at `$XDG_STATE_HOME/scoot/look/scoot.toml` (or
`$SCOOT_LOOK_STATE/look/scoot.toml`); a reload with a dangling link or an
unreadable fragment there is refused whole, keeping everything.

**A look-scoped reload** (`scoot msg reload --look`, driven by
`scoot look apply`) swaps only what the look layer supplies — appearance
and wallpaper — merged under a snapshot of the file from the last
successful plain load. It never re-reads the file, so it never applies
pending edits, spawns no autostart entry and swaps no binds; a
half-written file cannot refuse it. A missing or broken layer refuses it
whole, keeping everything. `SIGHUP` always runs the plain reload. No Nix
is involved on either path: the snapshot is the plain file, the layer is
plain rendered output, and both reloads work the same on a Nix install
and a plain binary install.

**Refused, pending a restart:** `[tty] gpu`, `[renderer]
backend`, `[xwayland] enabled`, `[virtual_input] enabled` and
`[virtual_input] binds` — all five take effect on restart, and each
refusal says so. A reload
that cannot load the file answers an `error` instead, keeps the
running config untouched, and logs — never defaults, never a
half-applied session, never an exit.

Two guarantees hold across every reload: a key held across a `[binds]`
rebuild (or a `[global_shortcuts]` re-resolve) neither wedges nor drops; and under `--tty` a reload cannot
strip the `Ctrl+Alt+F1..F12` VT-switch recovery bindings — they are
layered back on last, overriding any colliding file bind with a
warning. A reload applies while the session is locked, except new
autostart entries (a spawned program at lock time could disclose onto
the locked session, so those wait for the first unlocked reload).

A successful reload drops every live `scoot msg output-scale`
scale: the outputs go back to the file's scales (see
[Outputs](./outputs.md)), whether or not the file changed. Each dropped
scale the file disagrees with is listed in `applied` as
`outputs.<name>.scale`, the same name a changed `[[outputs]]` entry
reports under (a monitor unplugged since it was set is listed too: it
would have come back at that scale). A reload that cannot load the file
keeps them.

> **Symptom:** a reload changed nothing and both lists are empty.
> That means "the reload changed nothing it was asked to" — the file
> and the session already agree, and no live `output-scale` scale was
> left to drop. Edit first, then reload.

## Autostart

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `commands` | array of strings | `[]` | new `spawn` entries run once each | Action strings to run once each, in file order, at session startup — before the `--` command. |

Each entry is an action string — something `scoot msg action` would
accept and a `[binds]` value could contain:

```toml
[autostart]
commands = [
    "spawn waybar",
    "spawn mako",
]
```

Fail-open per entry: a malformed entry is skipped with a warning
naming that entry; every other entry still runs, and the session
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
input. The same state is drivable over IPC
(`scoot msg output-power 1 off`, `scoot msg outputs` reporting it).

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
# the connector name `scoot msg outputs` lists.
# [[outputs]]
# name = "eDP-1"
# scale = 2.0
#
# [[outputs]]
# name = "DP-1"
# mode = "1920x1080"

# [renderer]
# Type string, default "auto". One of "auto" (the default: the GPU
# renderer on real hardware, the CPU renderer everywhere else; never fails
# startup), "cpu" (pixman; needs no graphics device), or "gpu" (GLES;
# scanout on --tty). Restart-only: a reload refuses a changed value.
# --renderer wins over this when both name one, and SCOOT_RENDERER beats
# this when --renderer is absent.
# backend = "auto"

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
"super+y" = "spawn foot -e htop"
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

# pavucontrol's app id is org.pulseaudio.pavucontrol (`scoot msg windows`
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

Next: [Layout](./layout.md) and [Appearance](./appearance.md) cover every field of both tables.
