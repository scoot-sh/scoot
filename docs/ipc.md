# Driving scoot over IPC

Everything a keybinding can do, and everything a user can type or click, is
also a request on a Unix socket. This page is the reference for scripts and
for agents doing computer-use tasks.

The client is `scootctl`; `scoot msg ...` is the same client kept as a
permanent alias on the compositor binary. Every `scootctl` example below
works with `scoot msg` in its place, byte for byte.

- [The socket](#the-socket)
- [Requests](#requests)
- [Actions](#actions)
- [`type` vs `key`](#type-vs-key)
- [What the replies carry](#what-the-replies-carry)
- [Rules an agent needs](#rules-an-agent-needs)
- [What the socket refuses](#what-the-socket-refuses)
- [Resource bounds](#resource-bounds)

## The socket

The socket can inject any keystroke, so it is a privileged channel: it lives
in `$XDG_RUNTIME_DIR/scoot.sock`, is created `0600`, and serves only
connections from the same user as the compositor. A missing
`$XDG_RUNTIME_DIR` is a one-line startup error.

Override the path with `$SCOOT_SOCKET` (read by the compositor and
`scootctl`) or with `--socket PATH` on the compositor. Running two
compositors at once means giving each its own socket.

`scootctl` opens one connection per invocation and closes it as soon as it
has its answer. A client that wants many requests should pipeline them on
*one* connection rather than open a connection per request — the
per-connection bounds below are what keep the socket fair, and reconnecting
resets them.

`scootctl` prints an `error` reply and exits non-zero. The client builds on
every platform -- `scootctl` is the macOS package -- so a Mac can drive a
compositor running in a VM.

## Requests

| Request | What it does |
| --- | --- |
| `version` | Version and IPC protocol of the running compositor — needs a session. `scootctl --version` (or `scoot --version`) answers locally with no session, printing the binaries' own line (`scoot <version> (ipc protocol <N>)`) so a client can check compatibility before connecting. |
| `outputs` | Every output's name, rectangle, usable rectangle and scale. |
| `windows` | Every window: id, app id, title, icon, focus, popup grab. |
| `action ACTION [ARGUMENT...]` | Run a layout action — see [Actions](#actions). |
| `reload` | Re-read the config file the session started from and re-apply what can be re-applied live (gap, appearance, keybindings) — see [configuration.md](configuration.md#reloading-the-config). Answers `reloaded` with applied-vs-refused field lists, or `error` (running config untouched) when the file cannot load or validate. |
| `screenshot [--output ID] [--out FILE]` | Capture the screen as PNG. Without `--out`, the PNG goes to stdout. `--output` names which output to capture; every output has a framebuffer of its own, so the capture is that output's own pixels. An id naming no output is refused rather than answered with another output's pixels. Omitting it always means the first output (id 1). |
| `pointer move X Y` | Move the pointer to logical coordinates. |
| `pointer click X Y [left\|right\|middle]` | Move, then press and release. |
| `pointer button left\|right\|middle press\|release` | Half a click, for drags. |
| `pointer scroll DX DY` | Scroll by a delta. |
| `key COMBO` | Press one key combination — see [`type` vs `key`](#type-vs-key). |
| `type TEXT` | Type text on the active keyboard layout. |
| `wait-idle [--quiet-ms N] [--timeout-ms N]` | Block until nothing on screen has redrawn for `--quiet-ms` (default 200), giving up after `--timeout-ms` (default 5000). |

```sh
scootctl windows
scootctl action focus-column left
scootctl reload
scootctl screenshot --out /tmp/shot.png
scootctl type "hello"
scootctl wait-idle --quiet-ms 200
```

## Actions

The same grammar `scootctl --help` prints -- and `scoot --help` embeds --
and the same one a config file's `[binds]` values use: one parser handles
all three.

```
focus-column|move-column|consume-or-expel   left|right
focus-window|move-window                    up|down
focus-workspace|move-window-to-workspace    up|down
focus-window-id ID | focus-workspace-index N | move-window-to-workspace-index N | cycle-column-width | close | spawn COMMAND... | quit
```

`focus-workspace-index N` and `move-window-to-workspace-index N` are 0-based; out of range does nothing (a move leaves the window where it is). `spawn` is
split on whitespace and not run through a shell, so an argument containing a
space can't be expressed this way. Every action is refused while the session
is locked.

## `type` vs `key`

**`scootctl type TEXT` types text the way a person would**, on whatever
keyboard layout the session is running: for each character it finds the key
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
  Sequences come from the session locale (`LC_ALL`, then `LC_CTYPE`, then
  `LANG`, as the compositor process sees them — the same source toolkits
  read). In every case the characters *before* the failure have already been
  typed: the request stops at the first character it can't type rather than
  rolling back.
- Keybindings apply to what it types, exactly as they would to a real
  keypress. That only matters for a bind with no modifiers, or one on Shift
  plus a key; if a character does hit a bind, the compositor logs a warning
  naming it rather than swallowing it silently.

**`scootctl key COMBO` is not the same.** It presses exactly the
combination named and holds exactly the modifiers named, nothing more. Name
the key as it is with nothing held, plus the modifiers: `shift+1`, not
`exclam`; `shift+a`, not `A`. A name this layout only carries above its
unmodified level is refused, because the key that carries it types a
*different* character when pressed bare — `scootctl key exclam` would press
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
| `scale` | The output scale; `rect` and `usable` are logical, screenshots are physical. |

**`windows`**, one entry per window, in layout order:

| Field | Meaning |
| --- | --- |
| `id` | The window id. What `action focus-window-id N` takes, and what follows the last dash of an `ext-foreign-toplevel-list-v1` identifier. |
| `app_id` | The toplevel's app id, or `""` before the toolkit has sent one. |
| `title` | The toplevel's title, same caveat. |
| `icon` | The freedesktop icon name the client committed through `xdg-toplevel-icon-v1`, or `null`. Read off the surface's current state when asked, so it is never stale. A client that supplied raw pixel buffers instead of a name reads as `null`. |
| `output` | The id of the output the window is on. |
| `rect` | Where the window is, in logical pixels — what you click. A window that is not visible still reports the frame it *would* have. |
| `visible` | `false` when the window is scrolled out of view or on an inactive workspace. |
| `focused` | Compositor *window* focus — not necessarily where keystrokes go; see below. |
| `popup_grab` | Whether this window's own popup tree holds the keyboard — see below. |

Every success reply also carries **`locked`**: the session-lock state it was
built under. An agent typing a password over IPC learns the unlock landed
from the very next reply.

`locked`, `usable`, `scale`, `icon` and `popup_grab` are additive and
defaulted — an older server omits them rather than bumping
`PROTOCOL_VERSION`, so read each asymmetrically. `locked: true` and
`popup_grab: true` are always truthful, while `false` means "no, *or* a
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
  "refused": ["output.scale (startup-only: clients were told the scale at bind time)"] }
```

`applied` names the fields re-applied live, `refused` the ones that
differed but cannot be (each with its reason). Both name only fields that
*differed*: two empty lists together mean the reload changed nothing it was
asked to. A reload that could not load or validate the file answers
`error` with the running config untouched (`scootctl` exits non-zero).

## Rules an agent needs

**Window focus and keyboard focus are separate.** `scootctl windows`'
`focused` flag, the focus ring and `scootctl action focus-*` all mean the
*window*. A layer surface holding the keyboard (a launcher like `fuzzel` or
`wofi`, a bar's search field) never appears there — what `windows` reports is
where focus returns to once that surface goes away. So if a launcher is up,
`type` and `key` go to the launcher, while `windows` still names the window
behind it. There is no IPC request that reports a layer surface.

The focus-family actions do take the keyboard back. `focus-column`,
`focus-window`, `focus-window-id`, `focus-workspace` and
`focus-workspace-index` each spend a click that had given a *click-focused*
(`on_demand`) layer surface the keyboard, so after one, keystrokes go to the
window `windows` reports as focused. The other actions — `move-*`, `close`,
`spawn`, `cycle-column-width`, `quit` — change arrangement rather than where
focus is reported to be, so they leave a deliberate keyboard placement alone,
as does a keybinding. An `exclusive` layer surface keeps the keyboard through
all of them, by protocol, until it unmaps.

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
for exactly this.

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

## What the socket refuses

Nine bounds an agent can actually hit. The first seven are refusals with a
reason — an ordinary `error` response — rather than a silent drop or a delay.
The last two can't be: one drops a peer that is by definition not reading its
socket, and the other shortens a wait rather than refusing it.

- **One request line may be at most 1 MiB.** Past that the connection is told
  so and closed; there is no resynchronizing mid-line.
- **`type` text is limited to 16,384 characters per request.** Each character
  becomes key events typed synchronously on the thread that serves every
  other client, so a megabyte of text would stall the whole compositor for
  seconds. Split the text across several `type` requests. A shell command
  line's worth costs under a millisecond and never notices this. Counted in
  characters, not bytes; even the longest encodings land far under the 1 MiB
  line limit, so this cap always fires first.
- **One screenshot per connection per 16 ms frame.** A capture costs a render
  and framebuffer read-back on the thread that serves every other client, so
  a second one inside the same frame is refused rather than queued. Retry
  after a frame. Note the reply order for that pair: the refusal is answered
  immediately, while the capture it follows is still encoding — so the
  *second* request's reply arrives *first*. A client pipelining screenshots
  matches replies by content, not by position.
- **One capture in flight per connection.** The PNG encode runs on a worker
  thread, so other connections are answered while it runs — but the capture's
  own reply still has to go out before any later reply on that same
  connection, or a client reading replies in request order sees them swap.
  Any other request arriving on a connection with a capture in flight is
  refused with a retry rather than answered out of order. `scootctl` sends
  one request per connection and never meets this; nor does a capture refuse
  one on *another* connection. A capture whose earlier replies are still
  going out is refused the same way — retry once the queue has drained.
- **Four captures in flight at once, across every client.** Each holds a full
  frame of raw pixels on its way through the worker, so past four the next
  capture is refused with a retry rather than queued without bound. The
  encode does not block the event loop; the render and read-back still cost
  the loop a moment per capture, so under N concurrent capturers a bystander
  waits longer than one capture's share.
- **At most 64 connections at once**, across every client. A 65th is refused
  with a message naming the limit and closed immediately, not queued behind
  the others. This is what keeps the per-connection bounds meaningful.
- **A newcomer under file-descriptor pressure is refused.** While fewer than
  128 fds stand free process-wide, a new IPC connection is refused with a
  message naming the pressure and closed immediately — no slot taken, living
  connections untouched. Retry in a moment: pressure lifts as soon as whoever
  is holding fds lets go, and ordinary use (an idle compositor holds 14 fds,
  a `foot` window 17) never comes near it.
- **A connection whose peer stops reading is dropped**, ten to twenty seconds
  after the last byte it took (the check runs on a deadline of its own).
  Nothing is sent when this happens — there is nobody reading to send it to;
  the connection simply closes. Replies that don't fit in the socket are
  queued and pushed out as the client reads; a client that takes no bytes at
  all for that long — the classic case being one that sends a request, does
  `shutdown(SHUT_WR)` and then never reads the answer — is treated as gone.
  Reading *slowly* is fine and is never given up on: the clock runs from the
  last byte that actually went out, not from when the reply was queued, so
  draining a multi-megabyte screenshot over a minute costs nothing.
- **`wait-idle` waits at most 60 seconds**, whatever `--timeout-ms` asks for.
  A longer request isn't refused, it's shortened: the answer comes back at
  the minute mark at the latest. A waiting `wait-idle` keeps its connection
  (and one of the 64 slots) for as long as it waits, and — uniquely on this
  socket — cannot notice its client dying while it waits. The default is 5
  seconds and the request is meant for hundreds of milliseconds. A capture in
  flight neither extends nor shortens a wait.

## Resource bounds

Sizes a client or a config supplies are bounded, so one greedy or buggy
client cannot exhaust the compositor for the others. Ordinary use sits orders
of magnitude below all of these; they matter if you are writing a client that
allocates in a loop.

| Bound | Value | What happens past it |
| --- | --- | --- |
| `wl_shm` pool size | 512 MiB each | Protocol error on `create_pool`. Four full-screen 8K frames' worth. |
| Live `wl_shm` pools per client | 128 | Protocol error on the excess `create_pool`. Bounds live pool objects and the address-space envelope — *not* fds or mappings, since a buffer outlives its pool and retains both. |
| Live `wl_buffer`s per client | 512 | Protocol error on the creating object, whatever created it (pool, dmabuf, single-pixel). Each surviving shm or dmabuf buffer is what retains a compositor fd and, for shm, its mapping; single-pixel buffers retain neither but are counted uniformly, because the hook can't observe buffer kind. |
| Process-wide free fds | 128 | While fewer stand free, a new Wayland connection gets an immediate EOF (there is no protocol channel for a reason) and a new IPC connection is refused with a message. A client already holding past 128 live buffers or 64 live pools is refused its next creation with the same protocol error, so a client under those graces is never refused for another client's greed. |
| Manager/list binds per client | 8 | Across the workspace, both window-list and display-management globals combined. The ninth bind is closed with `finished` (plus the `done` batching requires) and announced nothing — a greedy client costs itself its ninth subscription, never another client's. |
| Unredeemed activation tokens | 64 | Across all clients, expired ones swept first. A spawn past a full table simply gets no token. |
| Live capture frame objects per client | 16 | The protocol's own `duplicate_frame` error, which disconnects the client that overflowed. |

An imported dmabuf's mapping is the one thing the buffer count does not
bound, because it lives in the renderer's cache and outlives the `wl_buffer`
that carried it; it is released instead from the same buffer-destruction
hook, immediately and without waiting for a frame.

Config-supplied sizes are bounded the same way: a client's declared minimum
window size can't exceed the largest output's usable area on each axis, and
`gap` and `cursor_size` each have an upper bound as well as a lower one — see
[configuration.md](configuration.md).
