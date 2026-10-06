---
title: Keybindings
description: "Every default keybinding, grouped by what you want to do, plus how to rebind."
---

Every default binding, grouped by intent. `Super` is scoot's own modifier
throughout; directions are vim's <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>.
The rightmost column is the action string — the same grammar `scoot msg action`
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
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>/</kbd> | show the live keymap (this page, in a terminal) |

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
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>/</kbd> | Show the live keymap in a terminal pager | `show-keymap` |

Quit is deliberately <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>e</kbd>, not
<kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>q</kbd>: one slipped Shift away from
"close window", and a slip shouldn't end the whole session.

The desktop profile adds three groups on top of these defaults:
<kbd>Super</kbd>+<kbd>d</kbd> opens the app launcher and
<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> its run mode (PATH
executables beside the apps) — see
[the desktop profile](../desktop/index.md#launcher) — the Fn-row
hardware keys (brightness, volume and mute, mic mute, media) plus
`Print` screenshots and the `Super` desktop actions, and an
on-screen display over the volume, brightness and mic-mute keys —
see [the desktop profile](../desktop/index.md#hardware-keys-and-desktop-actions)
and [the OSD](../desktop/index.md#sound-brightness-keys-and-the-on-screen-display).

Under `--tty`, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd>
additionally switch VTs — always winning over config binds, so the recovery
path survives a bad config.

## Show the keymap

Forgot a chord? <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>/</kbd> opens the
live keymap in a terminal (the same `foot`
<kbd>Super</kbd>+<kbd>Return</kbd> opens) in a pager — every combo the
running session binds, where each came from, and the config binds that were
skipped with their reasons. It is the same answer `scoot msg binds` prints:

```sh
scoot msg binds
scoot msg binds --json
```

The table is the default view, grouped the way this page groups it;
`--json` is the same reply as JSON, for agents and scripts. Either way it
is the live merged result — defaults, your `[binds]`, and `--tty`'s VT
switches — never a re-read of the file. The bind itself never fires while
the session is locked, like any other non-hardware bind.

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
scoot msg reload
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

Two failure behaviors worth knowing, since both degrade rather than fail
startup: a bind that doesn't parse is skipped with a warning naming just
that bind (everything else still loads — and `scoot msg binds` lists it
with its reason); and two combo strings resolving to the same combination
(`"Super+H"` vs `"super+h"`) are *both* skipped — "last one wins" would be
run-to-run-unstable, so the colliding group is dropped instead.

To remove a default outright instead of rebinding its combo, unbind it:

```toml
[binds]
"super+h" = "none"
```

`none` is not an action — no such verb exists, so the spelling cannot
collide with a real bind — in the string form or the table form
(`{ action = "none" }`; flags beside one are ignored). The combo goes
unbound: the key forwards to the client like any other unbound key, and
the bind stops firing. Removing the line falls back to the default, as
before. `scoot msg binds` shows the removal as
`config (unbinds default: <old action>)`; an unbind with nothing to remove
warns and is listed as skipped. Unbinding is purely additive — no existing
config changes meaning.

A bind can also be a table with the action under `action` plus two
opt-ins — one entry carrying everything about one combo, rather than
a second list of combos elsewhere that could disagree with the action:

```toml
[binds]
"XF86AudioRaiseVolume" = { action = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+", repeat = true, allow_when_locked = true }
```

To hold volume-up and have it keep stepping, that is the whole recipe:
`repeat = true`. To have the key work on the lock screen too,
`allow_when_locked = true` beside it.

- **`repeat` re-fires the bind while its key is held** — after a 200 ms
  delay, then 25 times a second (the seat keyboard's own rate, so
  binds step exactly the way a held key repeats in a terminal). The
  timer exists only while such a key is held. `quit` and `close`
  never repeat, even when flagged — holding quit must never end the
  session. Flagging either warns and runs the bind once.
- **`allow_when_locked` lets a `spawn` bind fire while the session is
  locked** — volume, brightness and media keys from the lock screen.
  Anything else keeps today's refusal even when flagged, and `scoot
  msg action ...` stays refused while locked too: an IPC request
  carries an arbitrary command from whoever sent it, while a bind can
  only run its config-pinned command.

Both default off, so a plain `"combo" = "action"` string behaves exactly
as before: fire once, never locked. A table entry missing its `action`,
or a non-boolean flag, warns and falls back; an unknown field warns
and is ignored, while the rest of the entry applies.

Fullscreen and maximize, spelled out: `Super+f` covers the whole output
while its column is focused — gaps, ring and a bar's reserved strip
included — keeping its place in the strip (focus away and the view
scrolls on; focus back and it covers again). Surfaces on the `top`
layer hide under it; `overlay` and the lock screen stay above.
`Super+m` fills the usable area instead (bar visible, gaps and ring
kept). Both restore the layout exactly on leave; moving the window, or
focusing a window stacked in the same column, ends them.
