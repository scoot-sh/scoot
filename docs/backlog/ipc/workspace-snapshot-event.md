---
title: "A coalesced `workspace` event on `subscribe`, so a bar can show occupied workspaces without polling"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# A coalesced `workspace` event on `subscribe`

Filed 2026-09-29 (scootbar planning). Serves **daily-drive** (a bar's
workspace module) and, secondarily, **computer use** (an agent learns
"workspace 3 now has windows" without polling `windows`).

## The gap

`ext-workspace-v1` is the standard route and carries the list, positions and
the one `active` bit (`docs/protocols.md#workspaces-ext-workspace-v1`). It has
no bit for "this workspace holds windows", and no other standard protocol
maps a toplevel to a workspace. So a bar can draw `1 2 3` with the active one
lit, but cannot dim the empty ones. `scoot msg windows` has each window's
`workspace`, but reading it means polling, and `subscribe` has one event kind
today, `output` (`docs/ipc.md#events`).

## What to do

A new `workspace` kind on `subscribe`, per the existing versioning rule (a
server that lacks it answers an ordinary `error`, so a bar degrades to "no
occupancy" rather than failing). One event shape, a **full snapshot** per
output rather than deltas, so a subscriber that missed one is never wrong:

```json
{"type":"workspaces","output":1,"name":"DP-1","active":0,"counts":[2,0,1]}
```

`counts[i]` is the number of windows on workspace `i` (0-based, the same
numbering `windows` and `focus-workspace-index` use). Whether the count or a
plain occupied flag is worth the extra bytes is the one open shape question;
a bar wants only the flag, an agent probably wants the count.

## Bounds and cost

This rides the same choke point as `refresh_workspaces` (the only writer of
the ext-workspace state), so it should fire from where that already
diffs, and only when the snapshot changed. It must be:

- **Coalesced** to at most one event per output per frame tick, whatever the
  window churn (a client opening and closing popups-as-toplevels at its real
  maximum rate must not become an event stream).
- **Allocation-free on the send path**, or built once per changed snapshot
  and reused for every subscriber.
- **Cheap with no subscriber**: one branch, nothing built.
- Subject to the existing subscriber rules: a subscriber that stops reading
  is disconnected, never buffered without bound.

Benchmark the window-churn path before and after, subscriber attached and not.

## Not in this ticket

Layout state (focused column, window counts per column), keyboard layout,
fullscreen and `locked` events. Add a kind when a consumer needs it; each is
server work on a hot path.
