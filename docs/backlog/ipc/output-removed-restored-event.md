---
title: "IPC event for output removed / restored"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# IPC event for output removed / restored

Filed 2026-09-27 from
`docs/backlog/resolved/unplug-adopted-windows-visible-done.md`, which
scoped it out deliberately: with the unplug switch, the origin names and
the IPC `workspace` field landed, the remaining legibility gap is a push
notification for consumers that poll today.

## What it is

An event subscription in `scoot-ipc` (which has none yet -- that is the
larger half of this item) carrying, for "output removed" and "output
restored": the adopter, the adopted workspace range, and the adopter's
previous and new active workspace. With it, a user who wants a desktop
notification wires `notify-send` to it, and an agent learns about a
monitor leaving without polling `windows`.

## Constraints (from the parent ticket)

- It fires on every monitor standby too (a routine unplug to scoot). That
  is fine for an event a consumer can filter or debounce, and it is the
  difference from a notification pushed at the user unconditionally --
  which is why scoot still draws/sends nothing itself.
- `scoot-ipc` has no event subscription yet, so design that half first;
  do not bolt a one-off channel onto the request/reply socket.
- The payload should reuse the vocabulary the parent ticket added: the
  0-based workspace indices IPC already speaks, the adopter's previous
  view as the core records it, and the origin connector name.

## Done when

- A subscribed client learns about a removal and a restore with the
  adopter, the adopted range, and the adopter's previous/new active
  workspace, without polling.
- `docs/ipc.md` documents the subscription and the payload.
- Tests pin the payload across adopt, renumbering and restore, the way the
  parent ticket's protocol tests pin the names.
