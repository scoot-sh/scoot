---
title: scootlaunch troubleshooting
description: "Launcher symptoms and their fixes — panel, ranking, icons, dmenu and run."
---

The panel stays shut, the order looks wrong, an icon is a letter, or a
script fails. Each fix below names the command that proves it. What the
launcher does is in [Overview](./index.md); keys, config and theming are
in [The panel](./panel.md); every flag is in the
[CLI reference](./cli.md).

> **Symptom:** the panel won't open.

The panel needs two things beside the binary: the keyboard library and
a font. Give a file directly to rule out the font search:

```sh
scootlaunch --check
scootlaunch --font /path/to/font.ttf
```

`--check` validates the config file without opening (`launch.toml`
beside the bar's, or `--config PATH`; unknown keys are refused by
name). Without `--font` it tries a short list of well-known paths and
refuses, naming what it tried. A missing `libxkbcommon.so.0` refuses
the same way. Full paths and the per-distro packages are in
[Overview](./index.md#install). The panel holds no surface when
closed: a second `scootlaunch` while it is open dismisses it, so one
bind toggles.

> **Symptom:** clicking outside the panel does not close it.

On scoot a press outside closes `scootlaunch`'s panel, matched by its
layer-shell namespace (`scootlaunch`). Another compositor keeps the
keyboard on an exclusive surface and never tells the panel about the
click, so the panel stays until Escape or a second `scootlaunch`. On
scoot, a panel run with a different namespace, or a layer surface that
asks for `on_demand` keyboard, is not closed this way.

> **Symptom:** the panel opens on the wrong monitor.

The panel names no output. An output-less layer surface lands on the
output under the pointer, else the primary one — the same rule new
windows use. Move the pointer onto the monitor you want first: the
compositor picks the output once, when the surface maps, and does not
move it afterwards.

> **Symptom:** ranking surprises — the wrong app is first.

The query matches case-insensitively over name, generic name,
keywords and Exec basename, plus a frecency bonus that never outranks
a much better text match. An empty query orders by frecency, then
name. Ask what the ranker saw:

```sh
scootlaunch --query "fire" --json | jq '.[0] | {id, name}'
scootlaunch --list --all --json | jq '.[] | select(.hidden != null) | {id, hidden}'
```

The history lives at `$XDG_STATE_HOME/scootlaunch/history`
(`~/.local/state` by default); deleting it clears the bonus. A
`TryExec` reason means its helper is not installed or not executable;
an entry missing even from `--all` has no usable `Exec` and the run
says why on stderr. Dmenu mode keeps no history: an empty query shows
stdin order.

> **Symptom:** rows show letters instead of icons.

Letters are the fallback: no `svg` feature for SVG-only names, no icon
theme installed, or nothing resolved or decoded. Check the entry and
the theme:

```sh
scootlaunch --list --json | jq '.[] | select(.id == "org.mozilla.firefox.desktop") | {id, icon}'
```

Lookup follows the Icon Theme spec (configured theme, its `Inherits`
chain, `hicolor`, then base directories); a name with a slash is used
directly. A PNG wins when the theme ships one; SVG rasterizes only
when no PNG does. The theme lives in `<state>/look/launch.toml` under
`[icons]`. Bounds (long names, huge files, symlink loops, missing
files) all fall back to the tile, never a panic. Full rules are in
[The panel](./panel.md#icons).

> **Symptom:** a dmenu or run script fails.

Check the bounds first, then the flags:

```sh
printf 'wifi-a\nwifi-b\n' | scootlaunch --dmenu --prompt='wifi: ' --no-run-if-empty --only-match
scootlaunch --run
```

Stdin is read to EOF before the first frame (5 s bound on a writer
that never closes), at most 1 MiB and 65,536 lines, each line at most
4 KiB. `--only-match` makes Enter with no match do nothing;
`--index` prints the 0-based stdin position instead of the line.
`--run` lists every executable on `$PATH` (first directory naming a
basename wins, at most 65,536 names). Exit codes: 0 for a printed
pick or launched entry, 1 for a dismiss or an empty feed with
`--no-run-if-empty`, 2 for a usage error. Any fuzzel flag outside the
supported set exits 2 naming it. A second `scootlaunch --dmenu` while
one is open opens a second panel: dmenu instances neither listen for
nor send the single-instance toggle.
