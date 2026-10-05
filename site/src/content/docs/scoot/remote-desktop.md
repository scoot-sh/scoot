---
title: Remote desktop (VNC)
description: "Reach the session over VNC with wayvnc — the switch, the security model, and the lock rule."
---

Reach the session from another machine over VNC: run scoot
[headless](./backends.md#the-three-backends), attach
[wayvnc](https://github.com/any1/wayvnc) (0.9.0 or newer, for
`ext-image-copy-capture-v1`), and connect any VNC client. Capture comes
through [screen capture](./protocols.md#screen-capture-ext-image-copy-capture-v1);
pointer, clicks, scroll and typing come through the two virtual-input
globals below, which are **opt-in** (see the security model).

```sh
# One screen, with remote control switched on:
scoot --headless --outputs 1 --config ~/.config/scoot/config.toml
WAYLAND_DISPLAY=wayland-1 wayvnc -o headless 127.0.0.1 5900
```

with, in the config file:

```toml
[virtual_input]
enabled = true
```

Then connect a viewer (`vncviewer 127.0.0.1:5900`, or macOS Screen Sharing
for the DES-auth fallback) — or tunnel it over SSH and keep wayvnc on
localhost, which is what it listens on by default. wayvnc captures one
output (`-o`; `-a` for all of them) at up to `--max-fps` (default 30), and
forwards pointer motion, five buttons, wheel scroll and keystrokes,
including its own keyboard layout (`wayvnc -k de` types German positions
correctly). An idle wayvnc costs nothing measurable: no CPU on either side
until the screen changes or the pointer moves.

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[virtual_input] enabled` | bool | `false` | restart only | Offer the virtual-pointer (`zwlr_virtual_pointer_manager_v1`) and virtual-keyboard (`zwp_virtual_keyboard_manager_v1`) globals a remote-control tool like wayvnc needs to drive the session. Off unless asked: any same-uid client that binds them can type and click as the user, so with this off the globals are not advertised at all. |

What to know before pointing wayvnc at a session:

- **Off unless `[virtual_input] enabled`.** Any same-uid client that binds
  these globals can type and click as the user, so unlike the clipboard
  globals (which every same-uid process can already reach past) they are
  not advertised at all until the key is set — wayvnc then logs
  `Virtual Pointer protocol not supported by compositor` (or the keyboard
  half) and refuses to start, except with `--disable-input` for a
  view-only session. An allow-list would be theatre — scoot has no
  security-context support to tell a privileged client from any other (see
  the trust note at the top of [Protocols](./protocols.md)) — so the switch
  is the whole gate, and it takes effect on restart: a reload refuses
  changes naming that. The threat this answers is not a local process
  (which could read your files anyway) but the network: wayvnc listens on
  TCP, and turning this on is what lets a remote machine drive the session.
- **Nothing virtual delivers while the session is locked.** Motion,
  buttons, scroll, keys and modifiers are dropped — a remote client cannot
  type a password into the lock screen, blind or otherwise, and can never
  unlock. Keys and buttons a device holds when the lock engages are
  released first, so unlocking never inherits a stuck modifier. The lock
  screen itself keeps showing over VNC (captures see it, never the windows
  behind it).
- **Virtual keys type through the seat layout.** A virtual keyboard brings
  its own keymap, but clients decode against the seat's — so each key is
  translated by keysym (`z` on a German remote types `z`, not the `y` at
  that position), without ever changing the seat keymap or the physical
  keyboard's state. A keysym the seat layout has no key for (a German `ß`
  on a US seat) is dropped rather than mistyped. Virtual keys never run
  keybindings: they are forwarded to the focused window like text, so
  remote `Super` moves no windows (unlike some compositors, where a remote
  Super drives the local binds) — window management stays local, or over
  `scoot msg` (see [Requests](../scootctl/requests.md#requests)).
- **Absolute motion maps onto the named output, or the whole layout.**
  wayvnc names the output it captures (manager version 2), so the pointer
  lands where the remote screen shows it; a client that names no output
  (or one that went away) maps across the output union instead, clamped at
  the edges. Relative motion is unaccelerated. Scroll arrives framed per
  sequence with wheel detents intact, so a remote wheel click scrolls
  exactly one detent.
- **A device that disappears releases what it held.** Disconnecting (or
  destroying) a virtual keyboard or pointer with keys or buttons still
  down synthesizes their releases on whoever has focus — a vanished remote
  cannot leave a button stuck. The same sweep runs on a VT switch away.
- **Unknown buttons and layouts are dropped, corrupt ones disconnect.**
  Buttons past the five this compositor names are dropped with a debug log;
  an unknown keymap format or an unreadable keymap fd keeps the device's
  previous keymap rather than taking it offline. An out-of-range axis or
  axis source is the protocol's own `invalid_axis` / `invalid_axis_source`
  error, and a key or modifiers before any keymap is `no_keymap` — all
  three disconnect only the client that sent them.
- **No transient seats, no per-window capture.** wayvnc's
  `--transient-seat` and single-window (`-o` naming a toplevel) modes need
  `ext-transient-seat-v1` and the toplevel capture source manager, neither
  of which scoot advertises; run it against an output (or `-a`) on the
  session's own seat.

Troubleshooting, by symptom:

- *wayvnc exits with `Virtual Pointer protocol not supported`*: the
  session runs with `[virtual_input]` off (or was not restarted after
  turning it on). Either switch it on and restart, or run wayvnc with
  `--disable-input` for view-only.
- *The viewer shows the screen but typing does nothing*: the session is
  locked (see above), or no window has keyboard focus — click one first.
- *A key types the wrong character*: the seat layout (US, unless the
  session was started with another) has no key for that keysym — see the
  translation rule above. Keys present in both layouts are exact.
- *The pointer is in the wrong place on a multi-output session*: run
  wayvnc with `-o` naming the output being viewed, so its absolute motion
  maps onto that output rather than the union.
