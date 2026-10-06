---
title: Drive scoot over IPC
description: "The IPC socket, the rules agents need, and the map to requests, actions, events and screenshots."
---

Drive scoot from a script or an agent: the socket, the rules that keep automation honest, and where the rest lives. The binaries document themselves
for agents: `scoot msg --help` (topics `requests`, `actions`,
`exit-codes`, `environment`; `help <verb>` for one verb's row) and
`scoot msg --help --json` for the machine-readable form — see
[Generated CLI pages](../reference/cli.md) for the contract every binary meets. The client is `scoot msg`, documented here. Bring exact verbs: the socket takes what you say literally.

# Driving scoot over IPC

Everything a keybinding can do, and everything a user can type or click, is
also a request on a Unix socket. This page is the reference for scripts and
for agents doing computer-use tasks.

The client is `scoot msg`, part of the compositor binary itself — there is
no separate client to install. Every example below is a `scoot msg`
invocation.

## The socket

The socket can inject any keystroke, so it is a privileged channel: it lives
in `$XDG_RUNTIME_DIR/scoot.sock`, is created `0600`, and serves only
connections from the same user as the compositor. A missing
`$XDG_RUNTIME_DIR` is a one-line startup error.

Override the path with `$SCOOT_SOCKET` (read by the compositor and
`scoot msg`) or with `--socket PATH` on the compositor. Running two
compositors at once means giving each its own socket.

`scoot msg` opens one connection per invocation and closes it as soon as it
has its answer. A client that wants many requests should pipeline them on
*one* connection rather than open a connection per request — the
per-connection bounds below are what keep the socket fair, and reconnecting
resets them.

`scoot msg` prints an `error` reply and exits non-zero. The client builds on
every platform -- on a Mac `scoot` is client-only (the compositor is
compiled out) -- so a Mac can drive a compositor running in a VM.

## Rules an agent needs

**Window focus and keyboard focus are separate.** `scoot msg windows`'
`focused` flag, the focus ring and `scoot msg action focus-*` all mean the
*window*. A layer surface holding the keyboard (a launcher like `fuzzel` or
`wofi`, a bar's search field) never appears there — what `windows` reports is
where focus returns to once that surface goes away. So if a launcher is up,
`type` and `key` go to the launcher, while `windows` still names the window
behind it. There is no IPC request that reports a layer surface.

The focus-family actions do take the keyboard back. `focus-column`,
`focus-window`, `focus-window-id`, `focus-workspace` and
`focus-workspace-index` -- with or without `--output ID` -- each spend a click that had given a *click-focused*
(`on_demand`) layer surface the keyboard, so after one, keystrokes go to the
window `windows` reports as focused. A `focus-window-id` naming no window
is a miss: it answers `ok` and changes nothing, leaving the click -- and
the keyboard it placed -- alone. The other actions — `move-*`, `close`,
`spawn`, `cycle-column-width`, `set-column-width`, `toggle-fullscreen`, `set-fullscreen`, `toggle-maximize`, `set-maximized`, `quit` — change arrangement rather than where
focus is reported to be, so they leave a deliberate keyboard placement alone,
as does a keybinding. An `exclusive` layer surface keeps the keyboard through
all of them, by protocol, until it unmaps. The one exception is the `top`
layer under a fullscreen window: while a fullscreen window covers an output,
that output's `top`-layer surfaces are not drawn, so neither a click-focused
nor an `exclusive` one there holds the keyboard until the output is
uncovered again (`overlay` surfaces are unaffected).

**`popup_grab` is the subtler case of the same split.** An explicit
`xdg_popup.grab` routes every keystroke to the menu until it is dismissed,
and compositor focus still moves underneath it — focus keybindings keep
firing with a menu open, by design. So after such a keybinding, `windows` can
report window B as `"focused": true` while every key still reaches window A's
menu, possibly off-screen, with no error. Each window therefore reports
`popup_grab` while its own popup tree holds the keyboard:

```json
{ "id": 1, "focused": false, "popup_grab": true }
```

The rule: **if any window reports `"popup_grab": true`, keystrokes go to that
window's menu, not to the focused window** — wait for the menu to close
(Escape or a click dismisses it) before typing at anything else. `false`
means no grab, or a server predating the field. Two limits: a grab rooted at
a layer surface — a bar's own dropdown — belongs to no window and leaves
every window `false`; and while the session is locked there is never a grab
to report, because locking dismisses any open one and refuses new ones.

**Screenshots are physical pixels; layout coordinates are logical.**
`screenshot` captures the framebuffer at full physical resolution, while
`windows` and `outputs` report logical rectangles. Convert with `physical =
logical * scale`, rounded down where a rectangle's edge lands mid-pixel (the
logical size is `ceil(physical / scale)`, so a full-output `logical * scale`
can overshoot by under one pixel). `outputs` reports each output's `scale`
for exactly this -- and it is per output: with [`[[outputs]]`](../scoot/outputs.md)
entries two screens can run at different scales, so convert a window's
rectangle with the scale of the output the window is on (its `output`),
never with the first output's. X windows (`--xwayland`) follow the same rule: their
`rect` is logical and a click at a logical point lands on the X widget
drawn there, whatever the scale -- the X server's own pixels (`ceil(scale)`
per logical pixel by default, `floor(scale)` with `[xwayland] fractional = "light"`, see [protocols.md](../scoot/protocols.md#x-windows-in-the-layout))
are never what an agent reads or sends.

**A pointer lock freezes injected motion, and still answers `ok`.** While a
client holds an active pointer lock (`zwp_pointer_constraints_v1` — a game or
3D app), `pointer move` and `click` answer `ok` but move nothing: the lock
owns the pointer until its client releases it. Clicks still reach whatever
surface holds pointer focus. A session lock ends the freeze — locking
deactivates the held constraint, so injected motion and clicks reach the lock
surface exactly like a real mouse, and unlocking re-arms the game's lock. Do
not assume a lock you observed survives a session lock.

**`wait-idle` waits for nothing on screen to have redrawn**, and a bar
redraws on its own schedule. With a `waybar` clock ticking once a second, a
short `--quiet-ms` settles normally while a long one never does and times
out. Keep `--quiet-ms` below whatever your bar's own redraw interval is — the
same caveat an animated cursor carries.
