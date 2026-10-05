---
title: Keybindings
description: Every default keybinding, grouped by what you want to do, plus how to rebind.
---

Every default binding, grouped by intent. `Super` is scoot's own modifier
throughout; directions are vim's <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>.
The rightmost column is the action string — the same grammar `scootctl action`
takes and `[binds]` uses, so each row is also its own rebind recipe.

## Move around

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>h</kbd> / <kbd>l</kbd> | Focus column left / right | `focus-column left\|right` |
| <kbd>Super</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Focus window down / up | `focus-window up\|down` |
| <kbd>Super</kbd>+<kbd>Ctrl</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Focus workspace down / up | `focus-workspace up\|down` |
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
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Move window down / up | `move-window up\|down` |
| <kbd>Super</kbd>+<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>k</kbd> | Move window to workspace down / up | `move-window-to-workspace up\|down` |
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
