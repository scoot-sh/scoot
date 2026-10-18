---
title: "`scootnotify`: the notification daemon (epic)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-server, notify-popups, notify-control, notify-dnd-history-rules, notify-fullscreen-hold-spike, notify-bar-feed, notify-nix, notify-packaging, notify-default-flip"
milestone: "M7"
---

# `scootnotify`

Filed 2026-09-29 as a pointer, not a spec: it gets its own backlog and
design pass when it starts. Serves **daily-drive**.

## The plan (2026-10-12)

The design is [dev/design/scootnotify.md](../../../dev/design/scootnotify.md); what it shares with the
other apps is in [shell-apps.md](../../../dev/design/shell-apps.md). That answers this entry's open question on
the lock. No standard protocol tells a non-locker client the session is locked, and scootnotify
needs none: scoot draws no layer surface while locked, and a card's expiry clock starts at the
compositor's first presentation of it, so a notification that arrives while locked waits
unexpired.

Children, in order (each `blocked` names its predecessors):

1. [notify-prototype](resolved/notify-prototype-done.md): the go/no-go gate against mako, out of the workspace,
   before any extraction.
2. [notify-server](notify-server.md): the D-Bus server and model.
3. [notify-popups](notify-popups.md): cards, stack, lock and idle rules, the look layer.
4. [notify-control](notify-control.md), then [notify-dnd-history-rules](notify-dnd-history-rules.md).
5. [notify-bar-feed](notify-bar-feed.md), then [notify-nix](notify-nix.md),
   [notify-packaging](notify-packaging.md) and [notify-default-flip](notify-default-flip.md).

Experiment: [notify-fullscreen-hold-spike](notify-fullscreen-hold-spike.md).

Each milestone's resource table is published here, and kept rather than overwritten, until the
app gets its own backlog. The first targets are the [design's budgets](../../../dev/design/scootnotify.md#budgets),
measured against mako, dunst and swaync.

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

Theming contract (added 2026-10-12): read the look layer and implement a
`reload` request on the control socket (whole-file validation, keep last
good, an applied/refused reply); see the [live looks design](../../../dev/research/live-looks-design.md),
section 9. A "look changed" toast from scoot's `look` event is an optional
hook.
