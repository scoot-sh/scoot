---
title: "`scootnotify`: the notification daemon (epic)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-prototype, notify-server, notify-popups, notify-control, notify-bar-feed, notify-nix, notify-packaging, notify-default-flip"
milestone: "M7"
---

# `scootnotify`

Filed 2026-09-29 as a pointer, not a spec: it gets its own backlog and
design pass when it starts. Serves **daily-drive**.

Its own binary, not a bar module. Outline, from the planning:

- Owns `org.freedesktop.Notifications` on the session bus via the shared
  [D-Bus client](resolved/dbus-client-done.md).
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

Same standards apply: lowest resource use of its class, a resource ratchet
against `mako`, `dunst` and `swaync`, docs in the same PR, and its own Nix
module.

## The plan (2026-10-12)

The design pass happened: [dev/design/scootnotify.md](../../../dev/design/scootnotify.md), with the parts it
shares in [shell-apps.md](../../../dev/design/shell-apps.md). It answers this entry's open question. No
standard protocol tells a non-locker client the session is locked (scoot's IPC `lock_changed`
exists for that, `site/src/content/docs/msg/events.md:178-181`), and scootnotify needs
neither: scoot draws no layer surface while locked, and a card's expiry clock starts at the
compositor's first presentation of it (the spec's "since the display"). A notification
that arrives while locked therefore waits, unexpired, with no IPC.

Children, in order (each `blocked` names its predecessors):

1. [notify-prototype](notify-prototype.md): the go/no-go gate against mako.
2. [notify-server](notify-server.md): the D-Bus server and model, headless.
3. [notify-popups](notify-popups.md): cards, stack, lock and idle rules, outputs.
4. [notify-control](notify-control.md): `msg`, JSON, `subscribe`/`wait`, DND, history, rules.
5. [notify-bar-feed](notify-bar-feed.md): the bar's `notifications` push module, natively.
6. [notify-nix](notify-nix.md): modules, Stylix, look; the opt-in profile switch.
7. [notify-packaging](notify-packaging.md), then [notify-default-flip](notify-default-flip.md).

Shared prerequisites: [scootui-extract-draw](scootui-extract-draw.md),
[scoot-dbus-extract](scoot-dbus-extract.md), [scootui-extract-wayland](scootui-extract-wayland.md).

## Resource ratchet

Each milestone's table is published here, and kept rather than overwritten, until the app gets
its own backlog. The [design's budgets](../../../dev/design/scootnotify.md#budgets) are the first targets, and
lean mako, dunst and swaync are the competitors.
