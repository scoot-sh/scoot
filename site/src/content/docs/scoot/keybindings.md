---
title: Keybindings
description: "Every default keybinding, grouped by what you want to do, plus how to rebind."
---

Every default binding, grouped by intent. `Super` is scoot's own modifier
throughout; directions are vim's <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>.
The rightmost column is the action string — the same grammar `scoot msg action`
takes and `[binds]` uses, so each row doubles as a rebind recipe.

## Cheat sheet

The whole map on one card:

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
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>t</kbd> | next look (wraps; nothing while locked) |

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
creates one nor falls back. (Empty workspaces are dropped, so an index
is only valid against the list it was read from.)

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

## Looks

| Keys | Does | Action |
|---|---|---|
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>t</kbd> | Next look (wraps through the catalog) | `spawn scoot look next` |

The bind does nothing while the session is locked. The desktop visibly
changes on each step; `scoot look current` names the active look. See
[Looks](../looks/index.md#cycle-and-preview) for `prev`, `random` and
timed previews.

Quit is <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>e</kbd> rather than
<kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>q</kbd>: one slipped Shift away from
"close window", and a slip shouldn't end the whole session.

The desktop profile adds three groups on top of these defaults:
<kbd>Super</kbd>+<kbd>d</kbd> opens the app launcher and
<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> its run mode (PATH
executables beside the apps) — see
[the desktop profile](../desktop/launcher.md) — the Fn-row
hardware keys (brightness, volume and mute, mic mute, media) plus
`Print` screenshots and the `Super` desktop actions, and an
on-screen display over the volume, brightness and mic-mute keys —
see [the desktop profile](../desktop/keys.md)
and [the OSD](../desktop/sound.md).

Under `--tty`, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd>
additionally switch VTs — always winning over config binds, so the recovery
path survives a bad config.

## Show the keymap

<kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>/</kbd> opens the
live keymap in a terminal: every combo the running session binds, where
each came from, and the config binds that were skipped with their reasons.
It is the same answer `scoot msg binds` prints:

```sh
scoot msg binds
scoot msg binds --json
```

The table is the default view, grouped the way this page groups it;
`--json` is the same reply as JSON, for agents and scripts. Config rows
sort by combo, so the JSON order is stable from run to run. Either way it
is the live merged result — defaults, your `[binds]`, and `--tty`'s VT
switches — never a re-read of the file. The bind itself never fires while
the session is locked, like any other non-hardware bind.

Which terminal opens follows your setup, in this order: `xdg-terminal-exec`
when it is on `PATH` (the desktop-preferred terminal); otherwise the
`Super+Return` bind's terminal when that bind is a `spawn` — rebind
`Super+Return` to `ghostty` and the keymap opens in ghostty too, with its
own arguments; otherwise `foot`, the default terminal. Inside, the
keymap pages through `less` when it is installed, and prints plainly and
waits for Enter when it is not. With no terminal to open,
the compositor logs one warning naming what it tried and does nothing.

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
`[xwayland] enabled` and `[virtual_input]` need a restart, and a reload
names them instead of silently ignoring them. A mistake never stops
scoot from starting: it logs the problem and uses the default.

Want the whole commented file? `scoot --print-default-config --write`
writes it once (and refuses rather than overwriting).

## The bind grammar

A `[binds]` entry is `"combo" = "action string"`. A combo is
`modifier+modifier+...+key` (`super+shift+h`), or a bare key with no
modifier (`"Return" = "close"` — legal, intercepting every press of that
key with no modifier held). Whitespace around `+` is ignored. Modifier
names are case-insensitive: `ctrl`/`control`, `shift`, `alt`,
`super`/`logo`/`meta`/`cmd`. The key is an xkb keysym name, tried
exactly then case-insensitively (or one of the four [scroll directions](#scroll-binds)) — and binds match a key's *unshifted*
symbol, so `"A"` means plain `a`, exactly like `"a"`: write
`"shift+a"` for the Shift chord.

Two failure behaviors, both degrading rather than failing
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

To hold volume-up and have it keep stepping: `repeat = true`. To have
the key work on the lock screen too: `allow_when_locked = true` beside it.

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

`Super+f` covers the whole output
while its column is focused — gaps, ring and a bar's reserved strip
included — keeping its place in the strip (focus away and the view
scrolls on; focus back and it covers again). Surfaces on the `top`
layer hide under it; `overlay` and the lock screen stay above.
`Super+m` fills the usable area instead (bar visible, gaps and ring
kept). Both restore the layout exactly on leave; moving the window, or
focusing a window stacked in the same column, ends them.

## Scroll binds

A bind can name a scroll direction instead of a key: `scroll-left`,
`scroll-right`, `scroll-up` or `scroll-down`, with any modifiers.

```toml
[binds]
"super+scroll-left"  = "focus-column left"
"super+scroll-right" = "focus-column right"
"super+scroll-up"    = "focus-window up"
"super+scroll-down"  = "focus-window down"
```

This is how a mouse wheel, a tilt wheel or a two-finger touchpad scroll
moves the strip. It works anywhere scoot sees an ordinary pointer: a
VM guest (vfkit, UTM, QEMU) gets the host trackpad as a plain pointer with
no multi-finger gestures, but its two-finger scroll arrives. A remote
desktop served through wayvnc sends its scrolls through a virtual pointer
instead, which needs `[virtual_input] binds = true` (the
"Virtual pointers" note below). Scroll binds reload live.

- **Direction is what the apps see.** `scroll-down` is the direction that
  scrolls your apps down, with your natural-scroll setting already
  applied. There is no second switch here: if scrolling feels backwards, fix
  the device setting, or swap the two binds.
- **Steps.** A wheel fires once per click; a high-resolution wheel that
  reports fractions of a click adds them up to a click. A touchpad (or any
  source that reports motion, not clicks) fires once per `step` units
  of motion, `60` by default; see [`[scroll]`](./configure.md#scroll). What is
  left after a step carries into the next, so a long swipe fires at an even
  pace. One event fires at most 8 steps, and `close`, `quit` and
  `show-keymap` at most one, so a fast flick cannot run away.
- **Starting over.** The partial step is dropped when the direction
  reverses, when an event matches no bind (the modifier let go, or the
  other direction), when a touchpad scroll ends, and after 300 ms
  without an event.
- **The client never sees a scroll a bind took.** With the modifier held
  and a bind for that direction, the event is removed before any window
  receives it. With no matching bind, the scroll reaches the window exactly
  as it did before, so a bind on `super+scroll-down` leaves plain scrolling
  alone. A scroll that moves both ways is taken one axis at a time: bind
  only the vertical directions and the sideways half still reaches the
  window.
- **A touchpad scroll is all or nothing.** One that a bind took is taken
  to its end, even if you let go of the modifier or reverse partway, so
  the window never sees the middle of a scroll it did not see begin. One
  that already reached the window is never taken partway when you press
  the modifier mid-scroll.
- **Not while locked.** Like other binds, scroll binds do nothing on a
  locked session; the scroll goes to the lock screen. A `spawn` bind with
  `allow_when_locked = true` still fires.
- **Virtual pointers.** A scroll from a virtual pointer (wayvnc and other
  remote-control tools) runs scroll binds only with
  `[virtual_input] binds = true`, the same switch that lets virtual
  keyboards run key binds.
- **Flags.** `repeat` is ignored on a scroll bind (a warning says so);
  it already fires once per step.
- **Bare scroll binds.** `"scroll-down" = "..."` with no modifier takes
  every downward scroll everywhere; legal, rarely what you want.

A bad direction name (`"super+scroll-sideways"`) is skipped with a
warning naming the four valid ones, and listed in `scoot msg binds`. The
four names are not key names, so `scoot msg key super+scroll-down` is
refused as an unknown key. `scoot msg binds` lists scroll binds with
the combos spelled the same way. `scoot msg pointer scroll 0 15` is one
wheel click for testing; it holds no modifier, so over IPC only a bind
with no modifier can be driven.

No scroll binds are set by default, and the desktop profile adds none:
plain scrolling is the first thing apps need, and the right modifier for
sideways movement depends on what else your setup already puts on
<kbd>Super</kbd>.

## Touchpad gestures

A swipe with three fingers (the default; `swipe_fingers` in
[`[gestures]`](./configure.md#gestures) makes it four) moves the strip.
Two-finger scrolling is not a gesture and stays the window's. A swipe with
any other number of fingers is not scoot's, and scoot does not yet hand
multi-finger swipes to windows (it does not offer the pointer-gestures
protocol), so those do nothing.

- **Sideways.** The strip follows your fingers one to one, with the columns
  that were off screen sliding in. Lift and the nearest column takes the
  focus, aligned so its left edge sits at the usable edge (clamped to the
  strip, like every other scroll). A quick flick goes one column further in
  its direction, however short. The view then eases into place with your
  [`[animation]`](./configure.md#animation) curve (spring included); with
  animation off it snaps. Past the first or last column the strip stretches
  and springs back.
- **Up and down.** One step when the swipe ends, if it travelled far enough:
  the next or previous workspace, or window in the column, as `vertical`
  says. Nothing tracks the fingers.
- **One direction per swipe.** The first bit of travel decides whether it is
  sideways or vertical, and the other direction is ignored for the rest of
  the swipe.
- **`natural`.** With the default `true` the content follows the fingers: swipe
  left and the columns to the right come into view. `natural = false`
  reverses both directions. This is separate from the device's own
  natural-scroll setting, which governs two-finger scrolling only.
- **Whole swipes only.** A swipe is scoot's or the window's from its first
  touch to the last. Changing `swipe_fingers`, turning a direction off or
  locking the session mid-swipe never gives a window half of one. A claimed
  swipe that is cut short (a finger lifted early) settles back to where it
  began and changes nothing.
- **Not while locked, not over a fullscreen window.** On a locked session a
  swipe goes to the lock screen and nothing moves. With a fullscreen window
  covering the output, scoot leaves the swipe alone and no window receives
  it either, so nothing happens.
- **What is drawn is not where the windows are.** While you swipe, and
  while the view settles, the windows are drawn shifted. Clicks, `scoot msg
  windows` and focus go by where the windows will be once it settles.
- **The settle lands on the column.** The release fires `scroll-view-to-column`:
  the settled column takes the focus and the view lines up with its left edge,
  clamped to the strip's end like every other scroll.

> **Symptom:** nothing happens on a swipe in a VM or over remote desktop.
> The host keeps multi-finger swipes, so the guest never receives them (a
> vfkit, UTM or VNC guest sees a plain pointer). Use
> [scroll binds](#scroll-binds) there.

Swipes come from libinput, so they work on `--tty` sessions. A `--nested`
scoot does not receive its host's swipes; use scroll binds there too.

## Global shortcuts

Some apps want a key that works no matter which window holds focus — a
push-to-talk key, a record toggle. Those live outside `[binds]`: the app
registers its shortcut over IPC, and *you* assign the chord, in
`[global_shortcuts]` keyed by `app-id:shortcut-id`:

```toml
[global_shortcuts]
"org.example.recorder:toggle-record" = "super+ctrl+r"
```

The app's own preferred trigger is a hint only — stored and listed, never
auto-bound. Nothing fires until you bind it, either here or with
`scoot msg shortcut-bind`:

```sh
scoot msg shortcuts
scoot msg shortcut-bind org.example.recorder toggle-record super+ctrl+r
scoot msg shortcut-bind org.example.recorder toggle-record reset
scoot msg subscribe global-shortcut
```

Rules, in one place: compositor binds always win (a chord the live
keymap holds stays listed but `conflicted` and never fires -- and two
registrations sharing one chord collide the same way: the loser stays
listed but `conflicted` and never fires); nothing
fires while the session is locked (a press while locked is forwarded, no
`Activated`; a release pairing a pre-lock press still closes with
`Deactivated`); key repeat never re-fires; and a
release is always swallowed for a press that was intercepted, even
across a focus change. Registrations die with the connection that made
them, so a dead app's chords go quiet on disconnect. A reload
re-resolves the file's chords against the live registrations (reported
as `global_shortcuts`) and tells subscribers to re-list.

Bounded: at most 64 shortcuts per connection and 512 compositor-wide,
`preferred_trigger` at most 256 bytes and `description` at most 1024
bytes -- past any of those `shortcut-register` is refused with an
ordinary error and nothing is stored. The caps also bound the
`shortcuts` list reply (the fullest possible reply fits under the 1 MiB
IPC line). Re-registering an id you already own is an upsert and never
counts against the count caps. `shortcut-bind` and `shortcut-unregister`
check the id exists but not that the caller owns it: any same-user
connection can bind or drop any registration, and any same-user
connection can subscribe to `global-shortcut` and receive presses. The
portal backend does not change that: it is the same user, not a sandbox
boundary (an IPC client can already inject keystrokes and screenshots,
so these primitives grant nothing new).

### Portal backend

Portal clients (OBS, chat apps) do not speak scoot IPC -- they speak
`org.freedesktop.portal.GlobalShortcuts`. That side is served by
`scoot-globalshortcuts-portal`, which owns
`org.freedesktop.impl.portal.desktop.scoot` and turns the compositor's
`shortcut_activated` / `shortcut_deactivated` / `shortcuts_changed`
events into the portal's `Activated` / `Deactivated` /
`ShortcutsChanged` signals. Install it, then bind as above; the app's
`BindShortcuts` call registers its shortcuts and your chord arrives as
the trigger.

```sh
# Nix (standalone; not part of the desktop profile)
programs.scoot-globalshortcuts-portal.enable = true
# Elsewhere: the deb, rpm and Arch packages ship the backend with its
# user unit, D-Bus activation file and portal file
scoot-globalshortcuts-portal --version
```

Rules, in one place: one portal session holds one app's shortcuts, and
a session's shortcuts die with it -- closing the session or killing the
app unregisters what the backend registered for it. If the compositor
restarts, the backend re-registers what it remembers (your chords come
back from `[global_shortcuts]`) and tells clients to re-list. Asking
for the shortcut configuration UI (`ConfigureShortcuts`) is an
acknowledged no-op: assignment lives in your config file, so edit it
and the backend picks the change up on the next reload.
