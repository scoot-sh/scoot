---
title: "`scootnotify`: the notification daemon"
status: "open"
area: "scootbar"
priority: "low"
blocked: "dbus-client and extract-scootui"
---

# `scootnotify`

Filed 2026-09-29 as a pointer, not a spec: it gets its own backlog and
design pass when it starts. Serves **daily-drive**.

Its own binary, not a bar module. Outline, from the planning:

- Owns `org.freedesktop.Notifications` on the session bus via the shared
  [D-Bus client](dbus-client.md).
- Popups are layer-shell `overlay` surfaces created on demand and destroyed
  when empty, so an idle daemon holds no surface. Drawn with
  [`scootui`](extract-scootui.md).
- Talks to the bar over the bar's control socket (`push`): DND state and the
  unread count as a module, with a click to toggle DND.
- Scoot draws and sends nothing itself
  (`docs/ipc.md#events`); `scootnotify` may optionally subscribe to scoot's
  `output` events (a monitor unplugged), as one more consumer of the
  existing stream.
- Session lock: notification content must not draw over a lock screen.
  Open: how it learns the lock state. Check whether any standard
  protocol tells a client (`ext-session-lock-v1` is for the locker itself); if
  none does, a scoot IPC event is the fallback. Settle this before building.

Same standards apply: lowest resource use of its class, a release gate
against `mako`, `dunst` and `swaync`, docs in the same PR, and its own Nix
module.
