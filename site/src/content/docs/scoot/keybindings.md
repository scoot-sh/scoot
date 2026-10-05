---
title: Keybindings
description: "Every default keybinding, grouped by what you want to do, plus how to rebind."
---

Every default binding, grouped by intent. `Super` is scoot's own modifier
throughout; directions are vim's <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>.
The rightmost column is the action string — the same grammar `scootctl action`
takes and `[binds]` uses, so each row is also its own rebind recipe.

## Cheat sheet

The whole map on one card — print it, or keep it on a second screen
until your fingers learn it:

| | <kbd>h</kbd> | <kbd>j</kbd> / <kbd>k</kbd> | <kbd>l</kbd> |
|---|---|---|---|
| **<kbd>Super</kbd>** | focus column left | focus window down / up | focus column right |
| **+ <kbd>Shift</kbd>** | move column left | move window down / up | move column right |
| **+ <kbd>Alt</kbd>** | consume / expel | — | consume / expel |
| **+ <kbd>Ctrl</kbd>** | — | focus workspace down / up | — |
| **+ <kbd>Ctrl</kbd>+<kbd>Shift</kbd>** | — | carry window to workspace down / up | — |

| | |
|---|---|
| <kbd>Super</kbd>+<kbd>1</kbd>…<kbd>9</kbd> / +<kbd>Shift</kbd> | go to workspace N / carry window there |
| <kbd>Super</kbd>+<kbd>comma</kbd> / <kbd>period</kbd> (+<kbd>Shift</kbd>) | focus output left / right, wrapping (carry with <kbd>Shift</kbd>) |
| <kbd>Super</kbd>+<kbd>r</kbd> / <kbd>f</kbd> / <kbd>m</kbd> | cycle width / fullscreen / maximize |
| <kbd>Super</kbd>+<kbd>Space</kbd> / +<kbd>Shift</kbd>+<kbd>Space</kbd> | focus floating vs strip / float or un-float |
| <kbd>Super</kbd>+<kbd>Return</kbd> / <kbd>q</kbd> / <kbd>Shift</kbd>+<kbd>e</kbd> | terminal / close window / quit |

## Move around

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>h</kbd> / <kbd>l</kbd> | Focus column left / right | `focus-column left\|right` |
| <kbd>Super</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Focus window down / up | `focus-window down\|up` |
| <kbd>Super</kbd>+<kbd>Ctrl</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Focus workspace down / up | `focus-workspace down\|up` |
| <kbd>Super</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Focus workspace 1–9 directly | `focus-workspace-index N` |
| <kbd>Super</kbd>+<kbd>comma</kbd> / <kbd>period</kbd> | Focus output left / right (wraps) | `focus-output-left\|right` |
| <kbd>Super</kbd>+<kbd>Space</kbd> | Move focus between floating windows and the strip | `toggle-floating-focus` |

Targeting a workspace that doesn't exist yet does nothing — it neither
creates one nor falls back. (Empty workspaces are dropped, so an index only
means something against the list it was read from.)

## Move windows

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>h</kbd> / <kbd>l</kbd> | Move column left / right | `move-column left\|right` |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Move window down / up | `move-window down\|up` |
| <kbd>Super</kbd>+<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Move window to workspace down / up | `move-window-to-workspace down\|up` |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Move window to workspace 1–9 | `move-window-to-workspace-index N` |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>comma</kbd> / <kbd>period</kbd> | Move window to output left / right (and follow it) | `move-window-to-output-left\|right` |
| <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>h</kbd> / <kbd>l</kbd> | Consume into / expel from a column | `consume-or-expel left\|right` |

## Shape windows

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>r</kbd> | Cycle the column's width | `cycle-column-width` |
| <kbd>Super</kbd>+<kbd>f</kbd> | Fullscreen on / off | `toggle-fullscreen` |
| <kbd>Super</kbd>+<kbd>m</kbd> | Maximize on / off (bar stays visible) | `toggle-maximize` |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> | Float the window, or put it back | `toggle-floating` |
| <kbd>Super</kbd>+drag | Move / resize a floating window (left / right button) | `[floating] modifier` |

## Launch and leave

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>Return</kbd> | Open a terminal (`foot`) | `spawn foot` |
| <kbd>Super</kbd>+<kbd>q</kbd> | Close the focused window | `close` |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>e</kbd> | Quit scoot | `quit` |

Quit is deliberately <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>e</kbd>, not
<kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>q</kbd>: one slipped Shift away from
"close window", and a slip shouldn't end the whole session.

Under `--tty`, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd>
additionally switch VTs — always winning over config binds, so the recovery
path survives a bad config.

## Change one binding

Bindings live in `[binds]` in `~/.config/scoot/config.toml`: `"combo" =
"action string"`. Combos are `modifier+modifier+...+key`
(`super+shift+h`); names are case-insensitive; a capital letter names the
*unshifted* key, so Shift chords spell Shift out. To open a different
terminal:

```toml
[binds]
"super+Return" = "spawn alacritty"
```

Then apply it without restarting:

```sh
scootctl reload
```

Keybindings reload live — like the layout, outputs, appearance, floating
rules and autostart. Only `[tty] gpu`, `[renderer] backend`,
`[xwayland] enabled` and an output's `mode` need a restart, and a reload
says so by name instead of silently ignoring them. A mistake never stops
scoot from starting: it logs the problem and uses the default.

Want the whole file, commented? `scoot --print-default-config --write`
writes it once (and refuses rather than overwriting).

## The bind grammar

A `[binds]` entry is `"combo" = "action string"`. A combo is
`modifier+modifier+...+key` (`super+shift+h`), or a bare key with no
modifier (`"Return" = "close"` — legal, intercepting every press of that
key with no modifier held). Whitespace around `+` is ignored. Modifier
names are case-insensitive: `ctrl`/`control`, `shift`, `alt`,
`super`/`logo`/`meta`/`cmd`. The key is an xkb keysym name, tried
exactly then case-insensitively — and binds match a key's *unshifted*
symbol, so `"A"` means plain `a`, exactly like `"a"`: write
`"shift+a"` for the Shift chord.

Two failure behaviors worth knowing, since both fail silently rather
than as a startup error: a bind that doesn't parse is skipped with a
warning naming just that bind (everything else still loads); and two
combo strings resolving to the same combination (`"Super+H"` vs
`"super+h"`) are *both* skipped — "last one wins" would be
run-to-run-unstable, so the colliding group is dropped instead. There
is no "unbind" action: a user bind on a combo with a default simply
replaces it, and removing a bind from the file falls back to its
default (or to unbound).

Fullscreen and maximize, spelled out: `Super+f` covers the whole output
while its column is focused — gaps, ring and a bar's reserved strip
included — keeping its place in the strip (focus away and the view
scrolls on; focus back and it covers again). Surfaces on the `top`
layer hide under it; `overlay` and the lock screen stay above.
`Super+m` fills the usable area instead (bar visible, gaps and ring
kept). Both restore the layout exactly on leave; moving the window, or
focusing a window stacked in the same column, ends them.
