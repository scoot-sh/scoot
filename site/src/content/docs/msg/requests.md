---
title: Requests and replies
description: "Every IPC request, type vs key, and what outputs, windows, keyboard and reload replies carry."
---

Ask the compositor questions and tell it to act: every request, `type` vs `key`, and what the replies carry. All of these work as `scoot msg <request>` or `scoot msg <request>`.

## Requests

| Request | What it does |
| --- | --- |
| `version` | Version and IPC protocol of the running compositor — needs a session. `scoot msg --version` (or `scoot --version`) answers locally with no session, printing the binaries' own line (`scoot <version> (ipc protocol <N>)`) so a client can check compatibility before connecting. |
| `outputs` | Every output's name, rectangle, usable rectangle, scale and power state. |
| `windows` | Every window: id, app id, title, icon, output, workspace, adoption, focus, popup grab. |
| `keyboard` | The active keyboard layout's name and index — what a layout indicator shows, and which layout the next `type` will produce. |
| `locked` | Whether the session is locked — the side-effect-free lock probe, answered locked or not (`{"type":"locked","locked":false}`). What the desktop clipboard asks before recording, and what tells an agent whether injected input would reach the lock screen instead of the desktop. |
| `binds [--json]` | The live keymap: every combo in canonical spelling, its action string, where it came from, its `repeat`/`allow_when_locked` flags, plus the config binds that were skipped with their reasons — see [`binds`](#binds). The human view is an aligned table grouped the way the [keybindings page](../scoot/keybindings.md) groups it; `--json` is the same reply as JSON, for agents. Answered locked or not. |
| `output-power ID\|all on\|off` | Switch an output's panel off or on — the IPC half of `zwlr_output_power_v1` (see [protocols.md](../scoot/protocols.md#screen-power)), for agents and scripts that do not speak Wayland. `all` powers every output at once; an unknown id is refused with an error. Session-level like `outputs`, not an action: it applies while locked (the idle cycle is off-after-lock, on-at-resume). On the wire an additive request tag (`{"type":"output_power","output":1,"powered":false}`, `output` omitted for `all`); `PROTOCOL_VERSION` did not change. |
| `output-scale ID\|NAME SCALE\|reset` | Set an output's scale live, by id or connector name (`DP-1`), or `reset` it to the config file's. The scale takes the config's range (0.5 to 4, resolved to 1/120 like the file's), but a value outside it, or not a number, is refused with the reason instead of clamped; an unknown id or name is refused too. Runtime state, like `output-power`: it beats the file's scale for that connector (a replugged monitor comes back at it), and a successful `reload` or a restart goes back to the file's scales. A repeat set, or a reset with nothing to reset, changes nothing. Applies while locked; refused under `--nested`, where the host owns the scale. What the [display profiles](../desktop/index.md#displays) watcher drives. On the wire an additive request tag: `output` is a number (id) or a string (name), and `scale` a number or `null` for reset, required either way (`{"type":"output_scale","output":"DP-1","scale":1.5}`); it is answered with the existing `ok`, so the request itself needs no newer protocol (an older server refuses the unknown tag with an `error`). |
| `action ACTION [ARGUMENT...]` | Run a layout action — see [Actions](#actions). |
| `reload` | Re-read the config file the session started from and re-apply what can be re-applied live (layout, output scale -- the default and each `[[outputs]]` entry's, appearance, keybindings, new autostart spawn entries; an entry's `mode` is refused as `outputs.<name>.mode`, pending a restart) — see [configuration.md](../scoot/configure.md#reloading-the-config). A successful reload also drops every live `output-scale` scale, and each one the file disagrees with is listed in `applied` as `outputs.<name>.scale`. Answers `reloaded` with applied-vs-refused field lists, or `error` (running config untouched, live scales kept) when the file cannot load or validate. |
| `screenshot [--output ID] [--out FILE] [--no-cursor]` | Capture the screen as PNG. Without `--out`, the PNG goes to stdout. `--output` names which output to capture; every output has a framebuffer of its own, so the capture is that output's own pixels. An id naming no output is refused rather than answered with another output's pixels. Omitting it always means the first output (id 1). The pointer is drawn in unless `--no-cursor` — see [The pointer in a screenshot](#the-pointer-in-a-screenshot). |
| `pointer move X Y` | Move the pointer to logical coordinates. |
| `pointer click X Y [left\|right\|middle\|back\|forward]` | Move, then press and release. |
| `pointer button left\|right\|middle\|back\|forward press\|release` | Half a click, for drags. |
| `pointer scroll DX DY` | Scroll by a delta. Sent as a wheel scroll that also carries its detents (eight v120 units per delta unit, so `pointer scroll 0 15` is exactly one detent) -- clients that only listen for steps still see it. |
| `key COMBO` | Press one key combination — see [`type` vs `key`](#type-vs-key). |
| `type TEXT` | Type text on the active keyboard layout. |
| `wait-idle [--quiet-ms N] [--timeout-ms N]` | Block until nothing on screen has redrawn for `--quiet-ms` (default 200), giving up after `--timeout-ms` (default 5000). |
| `subscribe [EVENT...]` | Dedicate this connection to events of the named kinds (`output`, `keyboard`, `workspace`, `lock`; naming none is refused — bare `scoot msg subscribe` sends `output`), streaming them until the session ends or drops the subscription — see [Events](#events). A fresh `keyboard` subscription starts silent, so issue one `keyboard` query for the baseline and listen for changes after it. A fresh `workspace` subscription starts silent too — read `windows` once for the baseline and apply snapshots after it. A fresh `lock` subscription starts silent too — read `locked` once for the baseline and apply changes after it. |

```sh
scoot msg windows
scoot msg action focus-column left
scoot msg reload
scoot msg screenshot --out /tmp/shot.png
scoot msg type "hello"
scoot msg keyboard
scoot msg locked
scoot msg binds
scoot msg wait-idle --quiet-ms 200
scoot msg subscribe
scoot msg subscribe keyboard
scoot msg subscribe lock
```

## `type` vs `key`

**`scoot msg type TEXT` types text the way a person would**, on whatever
keyboard layout the session is running (ask `scoot msg keyboard` which one
that is): for each character it finds the key
that carries it and holds down whatever modifiers that key's level needs —
Shift for `A` or `!`, AltGr for a German layout's `@` — so a client receives
the same key *and* modifier events it would see from a real keyboard, not
just a bare keysym. `\n` and `\t` are sent as `Return` and `Tab`.

A character no single keypress produces goes through a second path: the
two-key dead-led sequence from the session-locale compose table (`dead_acute`
then `e` for `é` on a German layout), pressed as the two keypresses a person
would type, each with its own level's modifiers. All 95 printable ASCII
characters type on every one of the fourteen swept Latin layouts (`us`,
`us(intl)`, `gb`, `de`, `de(neo)`, `fr`, `fr(oss)`, `es`, `it`, `pt`, `se`,
`no`, `dk`, `pl`).

- A character the active layout can't produce is an error naming it (``no key
  for `é` in this layout``). The compose table counts as well as the keymap:
  a character with no two-key dead-led sequence there stays refused — plain
  `us` carries no dead keys and no Compose key, so `é` is still "no key" on
  it, and three-key `Multi_key` sequences are not driven even on a layout
  with a Compose key. A character on an *inactive* layout group is refused
  too: nothing here switches the session's layout to go and find it. A
  character on a level the layout only reaches through a *locking or
  latching* modifier gets its own message (``[character] needs a modifier
  this layout only locks or latches``) — scoot will not press Caps Lock to
  type a capital, since that would leave it on for everything afterwards.
  Sequences come from your own XCompose file if you have one
  (`$XCOMPOSEFILE`, then `$XDG_CONFIG_HOME/XCompose`, then `~/.XCompose`),
  otherwise from the session locale's table (`LC_ALL`, then `LC_CTYPE`,
  then `LANG`, else `C`, as the compositor process sees them — the same
  source toolkits read; like `setlocale`, an empty variable counts as
  unset). In
  every case the characters *before* the failure have already been typed:
  the request stops at the first character it can't type rather than
  rolling back.
- Keybindings apply to what it types, exactly as they would to a real
  keypress. That only matters for a bind with no modifiers, or one on Shift
  plus a key; if a character does hit a bind, the compositor logs a warning
  naming it rather than swallowing it silently.

**`scoot msg key COMBO` is not the same.** It presses exactly the
combination named and holds exactly the modifiers named, nothing more. Name
the key as it is with nothing held, plus the modifiers: `shift+1`, not
`exclam`; `shift+a`, not `A`. A name this layout only carries above its
unmodified level is refused, because the key that carries it types a
*different* character when pressed bare — `scoot msg key exclam` would press
the `1` key and deliver `1`. Some characters can't be named as a combination
at all (`@` on a German layout needs AltGr, which `key` has no name for);
`type` is the one that works the modifiers out from the layout, and the one
to reach for when the goal is text rather than a chord. Modifiers resolve
from the active layout too — whichever key actually holds
Shift/Control/Alt/Super is what gets held (either hand's key, or the one a
layout option like `grp:lshift_toggle` left in place) — so a combo is refused
for its modifier only when no key on the layout can hold it.

## What the replies carry

**`outputs`**, one entry per output:

| Field | Meaning |
| --- | --- |
| `id` | The output's id, which a window's `output` names. |
| `name` | `HDMI-A-1`, `eDP-1`, … under `--tty` — the DRM connector name; `headless`, `headless-2`, … otherwise. |
| `rect` | The output's full rectangle, in logical pixels. "How big is the screen." |
| `usable` | The full output minus whatever a bar reserved at its edges (layer-shell exclusive zones) — where windows actually go. "Where can a window be." |
| `scale` | This output's own scale -- outputs need not share one (see [`[[outputs]]`](../scoot/outputs.md); `rect` and `usable` are logical, screenshots are physical. |
| `powered` | Whether the output is powered on — `false` while `output-power` (or `wlopm` over `zwlr_output_power_v1`) has it switched off. A powered-off output does no render work and takes no screenshots (refused, naming the recovery). |

**`windows`**, one entry per window, in layout order:

| Field | Meaning |
| --- | --- |
| `id` | The window id. What `action focus-window-id N` takes, and what follows the last dash of an `ext-foreign-toplevel-list-v1` identifier. |
| `app_id` | The toplevel's app id, or `""` before the toolkit has sent one. An X11 window's (under [`--xwayland`](../scoot/xwayland.md) is its `WM_CLASS` class -- `XTerm` for `xterm` -- falling back to the instance; X windows are listed like any other, with nothing on the wire to mark them. |
| `title` | The toplevel's title, same caveat. An X11 window's is `_NET_WM_NAME`, falling back to `WM_NAME`. An X title or class is cut at its first NUL byte (X allows one; a Wayland string cannot carry it). |
| `icon` | The freedesktop icon name the client committed through `xdg-toplevel-icon-v1`, or `null`. Read off the surface's current state when asked, so it is never stale. A client that supplied raw pixel buffers instead of a name reads as `null`, and so does every X11 window (`_NET_WM_ICON` is not read). |
| `output` | The id of the output the window is on. |
| `workspace` | Which workspace of that output the window sits on, 0-based — the same numbering `focus-workspace-index N` and `move-window-to-workspace-index N` take, so an agent can switch to the window's workspace without converting. (Bars see the 1-based twin over `ext-workspace-v1`.) |
| `adopted` | Whether the window's workspace was adopted from an unplugged monitor. With `origin`, what tells an agent where an unplugged monitor's windows went. |
| `origin` | Which connector the window's workspace was adopted from (`"DP-1"`), or `null` for a workspace that was never adopted. |
| `rect` | Where the window is, in logical pixels — what you click. That is the part of its layout slot the window has actually drawn: the slot's top-left corner, and the smaller of the slot and what the window last committed on each axis. Normally that is the whole slot. It is smaller for a window that draws less than it was given (a fixed-size dialog, a video player keeping its own size), and for the frame or two after its slot grows until the window's larger frame arrives. It is the same area the focus ring surrounds and rounded corners cut, and it never reaches past the slot. A window that has drawn nothing yet reports its whole slot. Its toplevel surface only: its open menus and other popups can draw outside `rect`. Whatever the window draws, only the part inside its own output's `rect` (`scoot msg outputs`) is shown and clickable: a column scrolled part-way past its output's edge is cut there (so is a menu crossing it, unless it lets the compositor adjust it — toolkit menus do — in which case it is flipped or slid back onto the window's output when it opens; see [protocols.md](../scoot/protocols.md#popup-menus-xdg_popup)), and a click past that edge lands on whatever the neighbouring output shows there — or on nothing, past the last output. A window that is not visible still reports the frame it *would* have — except a window stacked in the same column as a fullscreen one, which reports that fullscreen window's frame (it is behind it) until the fullscreen ends. The drawn-area rule applies only to visible windows. A window that is not visible reports its layout frame unchanged, because it draws nothing there. |
| `visible` | `false` when the window is scrolled out of view, on an inactive workspace, or hidden behind a fullscreen window (every other window on an output a fullscreen window covers, including windows stacked in its own column, and every floating window on it). A floating window is also `false` before its first frame, and while it is fullscreen without focus. |
| `focused` | Compositor *window* focus — not necessarily where keystrokes go; see below. |
| `popup_grab` | Whether this window's own popup tree holds the keyboard — see below. |
| `fullscreen` | Whether the window is fullscreen. While its column is focused it covers its output: `rect` equals that output's `rect` (for a window that draws its whole frame, as fullscreen windows do; see `rect`), and every other window on the output reports `visible: false`. Focused away, it keeps that size and sits in the strip where a column that wide would, one ordinary gap from its neighbours — it may still be partly `visible` beside the focused window, never overlapping it, and like any window is drawn and clickable only within its own output, however far its `rect` reaches past that output's edge. |
| `maximized` | Whether the window is maximized. While its column is focused (and no fullscreen window covers the output) it fills the output's usable area: `rect` equals that usable area minus the layout gap, and every other tiled window on the output reports `visible: false` (floating windows stay above it). Focused away, it keeps that size and sits in the strip where a column that wide would, one ordinary gap from its neighbours. Fullscreen wins while both hold. |
| `floating` | Whether the window floats above its workspace's strip: a dialog, transient or fixed-size window floated as it mapped, a `[[window_rule]]` match, or `toggle-floating`/`set-floating`. Its `rect` is where it really is: where it was last moved to (by a drag or `move-floating`), else centred on its parent (when that is visible on the same workspace) or its output, inside the output's `usable` area, at the size it drew. It is drawn above, and takes clicks before, every tiled window on its output. A floating window that has not drawn its first frame yet is `visible: false`, as is one on an inactive workspace or under a fullscreen window that covers the output. |

**`keyboard`**, the seat keyboard's currently effective layout (xkb group):

| Field | Meaning |
| --- | --- |
| `index` | The active group, 0-based — the same numbering `type` resolves each character in, so an agent reads off which layout its next `type` will produce. |
| `name` | The keymap's own name for that group (`us` reads as `"English (US)"`, `ru` as `"Russian"`) — what a bar shows. |

Read live off the compositor's keymap on every request: there is no
scoot-side copy to go stale. Read-only, like the event below — scoot has
no layout-switch bind, option or action, so nothing over IPC switches the
layout; the group moves only through the keymap's own mechanics (a toggle
key from the `XKB_DEFAULT_OPTIONS` the session started with, e.g.
`grp:caps_toggle`).

**`locked`**, whether the session is locked:

| Field | Meaning |
| --- | --- |
| `locked` | `true` while the session is locked, `false` otherwise — read live off the compositor's lock state on every request, so there is no copy to go stale. |

Unlike every `action`, this answers locked or not: it is the
side-effect-free probe the desktop clipboard runs before recording, and
what tells an agent whether injected keyboard and pointer input would
reach the lock screen instead of the desktop. A `focus-window-id` for an
id no window can hold used to serve as that probe; it spent an
`on_demand` layer surface's keyboard focus on every probe, and a miss now
leaves focus state alone — one `locked` round trip replaces two requests.
Read-only, like the event below: nothing over IPC locks or unlocks the
session.

```sh
$ scoot msg locked
{
  "type": "locked",
  "locked": false
}
```

**`binds`**, the live keymap as the running compositor uses it — what the
default `Super+Shift+/` bind opens in a terminal, and what an agent reads
instead of re-parsing the user's config file:

| Field | Meaning |
| --- | --- |
| `combo` | The combo in canonical spelling: modifiers in `super+shift+ctrl+alt` order, then the xkb keysym name (`super+shift+slash`). |
| `action` | The action string: the same grammar `scoot msg action` takes and `[binds]` uses (`focus-column left`, `spawn foot`, `show-keymap`; `change-vt 3` for the session-managed VT-switch binds, which have no config spelling). |
| `source` | Where the row came from: `default` (a built-in bind, unchanged), `config` (a combo the defaults leave unbound), `config (replaces default: <old action>)`, `config (unbinds default: <old action>)` (a config unbind — `action` names what was removed), or `session (VT switch)` (the `--tty` recovery binds). |
| `repeat` | Whether the bind re-fires while its key is held. Never true on a `default` row. |
| `allow_when_locked` | Whether a `spawn` bind fires while the session is locked. Only ever true beside a `spawn` action. |

One row per effective binding — built-ins first in their canonical order,
then config binds (fresh combos and overrides alike) sorted by combo, then
any session-managed rows — followed by one row per default a config unbind
removed. The config sort is what keeps `--json` stable from run to run: a
config file loads out of a hash map, whose order has no relationship to the
file's. A second list, `skipped`, holds every config entry that never made
it in, sorted by combo — each with its `bind` (as written), its `value` (in
TOML form), and its `reason` (a parse error, a colliding group, or an unbind
with nothing to remove).

```sh
$ scoot msg binds --json
{
  "type": "binds",
  "bindings": [
    {
      "combo": "super+h",
      "action": "focus-column left",
      "source": "default",
      "repeat": false,
      "allow_when_locked": false
    }
  ],
  "skipped": []
}
```

Derived by diffing the live table against the defaults on every request,
so it is always the merged result — never a re-read of the file. Like
`locked`, this answers locked or not: listing what keys do is not a
window-management operation (the `Super+Shift+/` bind itself never fires
while locked). A new reply variant, so this moved `PROTOCOL_VERSION`
8 → 9: an older client handed one would fail its decode, which can only
happen to a client new enough to have asked; a `binds` sent to an older
server answers an ordinary `error`.

Every success reply also carries a **`locked` field**: the session-lock state it was
built under. An agent typing a password over IPC learns the unlock landed
from the very next reply.

`locked`, `usable`, `scale`, `icon`, `popup_grab`, `fullscreen`,
`maximized` and `floating` are additive and defaulted — an older server omits them rather than bumping
`PROTOCOL_VERSION`, so read each asymmetrically. `locked: true`,
`popup_grab: true`, `fullscreen: true`, `maximized: true` and `floating: true` are always truthful, while `false` means "no, *or* a
server predating the field"; an all-zero `usable` means the same (fall back
to `rect`), and an omitted `scale` decodes as `1.0`.

**`reloaded`**, the answer to `reload`, is the exception that proves the
rule above: it is a new reply variant, not a defaulted field, and it moved
`PROTOCOL_VERSION` 2 → 3 (an older client handed one would fail its decode
— in practice only a client new enough to send `reload` ever receives one).
Read it asymmetrically from the other direction: a `reload` sent to a
server predating the request answers an ordinary `error`, not a kill — an
unknown request tag is a decode error the server answers and keeps serving.

```json
{ "type": "reloaded", "applied": ["layout.gap", "binds"],
  "refused": ["tty.gpu (takes effect on restart: the session already drives its device)"] }
```

`applied` names the fields re-applied live (including `autostart.commands`
when new spawn entries started -- a spawn entry that fails to start is
refused by name instead, and stays pending for the next reload -- and
`wallpaper` / `wallpaper.command` when the `[wallpaper]` section or its
`command` changed: handed to scootbg, whose outcome is in the compositor
log, not in this reply), `refused` the ones that
differed but cannot be (each with its reason: the two restart fields, a
non-`spawn` autostart entry by name, a locked-skipped autostart delta,
an unusable `[[window_rule]]` by its position in the file, or a
`[wallpaper]` section with a problem, named -- an unknown key inside
`[wallpaper]` is this refusal, not an `error`). Both name only
fields that *differed*: two empty lists together mean the reload changed
nothing it was asked to -- except an unusable window rule and a
`[wallpaper]` section with a problem, each refused on every reload that
finds it, since neither is ever in effect. A reload that could not load or validate the file answers
`error` with the running config untouched (`scoot msg` exits non-zero).
