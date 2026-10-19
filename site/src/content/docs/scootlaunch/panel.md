---
title: The panel
description: "Open, type, pick: keys, mouse, config, and theming for the scootlaunch panel."
---

![The launcher panel in moonrise, filtered to "term"](../../../assets/scootlaunch-moonrise.png)

Bare `scootlaunch` opens the panel: an overlay surface with exclusive
keyboard, centered on the output, 40 em wide with 10 rows by default.
Type to filter; Enter picks. The panel holds no surface when closed —
one process per open.

```sh
scootlaunch                          # open the panel
scootlaunch --font /path/to/font.ttf # open with an explicit typeface
scootlaunch --check                  # validate the config file and exit
```

Picking prints the chosen entry id and exits 0; dismissing exits 1.
Launching is the next ticket: the id is printed for now, not spawned.
The panel links `libxkbcommon.so.0` for its keyboard (the reference
XKB implementation); a scoot desktop already carries it.

## Keys

| Key | Action |
|---|---|
| typing | filters the list |
| Up / Down, Ctrl+P / Ctrl+N, Shift+Tab / Tab | move the selection |
| Page Up / Page Down, Ctrl+Home / Ctrl+End | by page, to the ends |
| Enter | pick the selection (prints its id) |
| Ctrl+Enter | run the typed text (prints it, for now) |
| Escape | dismiss (exit 1) |

Left/Right, Home/End, BackSpace, Delete, Ctrl+W, Ctrl+U and Ctrl+A
edit the query. A click picks the row; the wheel scrolls. A click on
the bare desktop does not dismiss in v1: with exclusive keyboard the
compositor holds focus, so the click never reaches the panel. A
second `scootlaunch` while the panel is open dismisses it, so one
bind toggles: no pid file, no abstract name, just a socket in
`$XDG_RUNTIME_DIR` with the peer's uid checked.

## Config

`$XDG_CONFIG_HOME/scoot/launch.toml` beside the bar's, or `--config
PATH`. Every key is optional; a missing file is the built-in
defaults. A bad file refuses loudly. `--check` validates without
opening.

| Key | Type / default | Example |
|---|---|---|
| `font` | path, none (first well-known file) | `font = "/usr/share/fonts/TTF/DejaVuSans.ttf"` |
| `width_em` | integer 10–200, `40` | `width_em = 50` |
| `rows` | integer 1–64, `10` | `rows = 8` |

Unknown keys are refused, never ignored.

## Theming

The panel reads `<state>/look/launch.toml` at every open: the
thirteen shell-app color roles plus `font`, `font-size`, `opacity`
and `radius`, merged per key as flags, then this file, then the look,
then the built-ins. A refused key keeps its default and is reported;
a missing layer is the defaults with one warning.

| Element | Role |
|---|---|
| Panel fill | `surface` (default `background`), at `opacity.popups` |
| Input and row text | `foreground` |
| Placeholder | `dim` |
| Matched characters | `match` (default `hover`) |
| Selected row fill / text | `selection` / `selection-text` (defaults `accent` / `background`) |
| Panel border | `focus` (default `accent`) |
| Caret, entry focus ring | `accent` |
| Corners | `radius` |
| Size | `font-size` (the look's popup size) |

The `font` key names the look's UI family. There is no fontconfig in
this stack: the panel draws with `--font PATH`, else the first usable
well-known file, as the bar does. Rows show the entry name with its
match highlighted; generic names, comments and icons arrive in later
tickets. A light look draws the same panel from the same roles:

![The launcher panel in music-desk, filtered to "ca"](../../../assets/scootlaunch-music-desk.png)

## Troubleshooting

> **Symptom:** the panel opens on the wrong monitor.

The panel names no output, and an output-less layer surface lands on
the compositor's primary today. The core
`layer-shell-default-output` ticket moves it under the pointer; until
then the panel carries no output logic of its own.

> **Symptom:** the panel refuses to start over a font.

Give it a file with `--font PATH` (a `.ttf` or `.otf`, such as
DejaVuSans.ttf). Without one it tries a short list of well-known
paths and refuses, naming what it tried.

> **Symptom:** the panel ignores the look.

The panel warns once per fallback on stderr: a dangling or missing
link means no look layer, and a refused key names itself. `scoot look
apply` re-renders the layer; the next open reads it.
