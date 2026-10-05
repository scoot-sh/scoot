---
title: Events
description: "Subscribe to output, keyboard, workspace and lock events over IPC."
---

Subscribe instead of polling: dedicate a connection to the event stream and learn about outputs, layouts, workspaces and the session lock as they change.

## Events

A connection that wants push notifications subscribes instead of polling.
`subscribe` names the event kinds it wants (`output`, `keyboard`,
`workspace`, `lock` — the four
kinds; naming none is refused, and bare `scoot msg subscribe` sends
`output`); the reply echoes the subscription; and
afterwards that connection carries events until the session ends:

```sh
$ scoot msg subscribe
{"type":"subscribed","events":["output"]}
{"type":"output_removed","output":2,"name":"DP-1","adopter":1,"adopted_start":2,"adopted_count":2,"adopter_prev_active":0,"adopter_active":2,"origin":"DP-1"}
{"type":"output_restored","output":3,"name":"DP-1","adopter":1,"adopted_start":2,"adopted_count":2,"adopter_prev_active":2,"adopter_active":0,"origin":"DP-1","moved":3}
{"type":"output_changed","output":1,"name":"DP-1","width":2952,"height":1660,"scale":1.5}
$ scoot msg subscribe keyboard
{"type":"subscribed","events":["keyboard"]}
{"type":"keyboard_changed","name":"Russian","index":1}
$ scoot msg subscribe workspace
{"type":"subscribed","events":["workspace"]}
{"type":"workspaces","output":1,"name":"DP-1","active":0,"counts":[2,0,1]}
$ scoot msg subscribe lock
{"type":"subscribed","events":["lock"]}
{"type":"lock_changed","locked":true}
```

`scoot msg subscribe` prints the answer, then one compact JSON object per
line per event, until killed or the connection ends — so a desktop
notification is one pipe away (`scoot msg subscribe | ... notify-send`), and
an agent learns about a monitor leaving without polling `windows`. It exits
0 at a clean end of stream; re-run to resubscribe.

Three rules, matching the request/reply contract beside them:

- **A subscribed connection serves no further requests.** After the
  `subscribed` answer it carries events only; any other request on it is
  refused with an error naming the rule. Open another connection for
  requests — they pipeline, so one is enough for any number of them.
- **Filtering is by kind, not by field.** The server sends every event of
  the subscribed kinds, and the client filters or debounces further
  itself. In particular the removal/restore pair below fires on every
  monitor standby too (a routine unplug to scoot) — that is accepted, and
  it is the difference from a notification pushed at the user
  unconditionally, which is why scoot still draws and sends nothing
  itself. `output_changed` fires only on an applied resize, never on
  standby (the mode is unchanged then), and a refused resize fires
  nothing.
- **The same socket and the same credentials.** There is no second channel:
  a subscriber connects to the same `0600`, same-user control socket every
  other client uses.

**`output_removed`** — an output was removed and its workspaces adopted:

| Field | Meaning |
| --- | --- |
| `output` | The removed output's id, as `outputs` reported it. |
| `name` | Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) — the identity a later restore matches on. |
| `adopter` | The output that adopted its workspaces, or `null` when none did. |
| `adopted_start` / `adopted_count` | The adopted block on the adopter: the 0-based workspace index it starts at, in the post-removal list, and how many workspaces it holds (0 when the removed output held no windows). |
| `adopter_prev_active` / `adopter_active` | The adopter's active workspace before and after the removal, 0-based — where it was already looking, or the adopted workspace the switch moved it to when focus was on the removed output. `null` with no live adopter to read off. |
| `origin` | Which connector the adopted workspaces are tagged as coming from (`DP-1`) — what bars show and what `windows` reports each adopted window adopted from. `null` when nothing was adopted. |

**`output_restored`** — an output came back under a matching identity (note
the fresh `output` id: ids are stable for the session, not across unplug
cycles). The adoption fields describe the record this restore consumed, as
the removal filed it; `moved` says how many still-open windows actually
went back — windows moved by hand or closed in between stay where they
are. `adopter_prev_active` / `adopter_active` are the adopter's view before
the restore and after it returns to its pre-adopt view (`null` when the
adopter itself is gone, e.g. a chained unplug whose middle monitor never
returned — then nothing moved either).

**`output_changed`** — an output's mode changed in place: the same
connector at a new framebuffer size, with the scale it keeps running at (a
mode change never changes the scale). This is the re-probe resize — a
`--tty` hotplug offering a new mode for a connector that stays connected
(a VM window moving between displays of different densities), or a
`--nested` host window being resized — not a removal: no workspace is
adopted, nothing moves, and no restore follows. A script watching density
recomputes the scale it wants from `width` / `height` and applies it with a
config reload (`output.scale` and `outputs.<name>.scale` apply live);
polling `outputs` gives the same numbers, this just says when to look. It
fires once per applied resize, after the windows are re-laid-out at the new
size; a resize the render target refuses fires nothing.

| Field | Meaning |
| --- | --- |
| `output` | The output's id, as `outputs` reports it. |
| `name` | Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) — the same string `outputs` names it by. |
| `width` / `height` | The new framebuffer size in physical pixels. |
| `scale` | The scale the output keeps running at — the scale to recompute *from*. |

**`keyboard_changed`** — the seat keyboard's effective layout (xkb group)
changed: the same `name` and `index` the `keyboard` query answers with
(see [What the replies carry](#what-the-replies-carry)), naming the layout
now in effect. No standard Wayland protocol reports this to an unfocused
client — `wl_keyboard` sends the keymap and the modifier group only to the
client holding keyboard focus, which a bar never does — so this event (and
the query) is the channel a layout indicator reads.

It fires once per change, never per keypress: typing on one layout sends
nothing, and one group switch sends exactly one event. Rapid successive
switches each send their own — every event names the layout in effect when
it was sent, so a reader that processes them in order ends where the
keyboard is. As with the query, this is read-only: nothing over IPC
switches the layout, and the bar's click action has nothing to call — the
group moves only through the keymap's own mechanics (a toggle key from the
`XKB_DEFAULT_OPTIONS` the session started with). Layout switching UI and
per-window layouts are out of scope.

A subscriber that stops reading is disconnected rather than buffered
without bound: past the same 1 MiB queued-reply bound a connection
observes, or with no byte leaving for the same 10-second stall window, the
compositor shuts the connection down and drops the subscription. Output
removal never waits for a subscriber. A client that disconnects itself
leaves no record behind.

**`workspaces`** — one output's workspace occupancy changed: which of its
workspaces hold windows, as a full snapshot rather than a delta, so a
subscriber that missed one is never wrong. What a bar's workspace module
draws (dimming the empty ones) without polling `windows`, and what tells an
agent "workspace 3 now has windows" the same way. No standard Wayland
protocol reports this to an unfocused client — `ext-workspace-v1` carries
the list, positions and the one `active` bit, but no "holds windows" bit,
and no other standard protocol maps a toplevel to a workspace.

| Field | Meaning |
| --- | --- |
| `output` | The output's id, as `outputs` reports it. |
| `name` | Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) — the same string `outputs` names it by. |
| `active` | The output's active workspace, 0-based — the same numbering `focus-workspace-index N` takes. |
| `counts` | One entry per workspace, in order: how many windows sit on it (`counts[i] > 0` is the occupied flag a bar draws; the number itself is what an agent reads). |

One event per output whose snapshot moved — a window opened, closed or
moved between workspaces, the active workspace switched, or an output
added (a removed output sends nothing: its removal is already an
`output_removed`, and there is no occupancy left to report). Coalesced to
at most one event per output per frame tick, however fast windows churn: a
client opening and closing windows at its maximum rate is one event per
tick, not an event stream. A fresh subscription starts silent, like
`keyboard` — read `windows` once for the baseline (the counts are the
histogram of each window's `workspace` on its `output`) and apply snapshots
after it — so subscribing never replays the unsubscribed interval as one
change.

**`lock_changed`** — the session locked or unlocked: the same `locked`
flag the `locked` query answers with (see [What the replies carry](#what-the-replies-carry)),
naming the state it moved to. What tells an agent "injected input now
reaches the lock screen" without polling, and what a clipboard watcher
leans on instead of its probe interval. No standard Wayland protocol
reports this to an unfocused client — `ext-session-lock-v1` events go
only to the lock client itself — so this event (and the query) is the
channel that watches the lock.

It fires once per transition, never per request: a lock that is refused
(another client holds the session) changes nothing and sends nothing, and
a takeover of an already-locked session is no transition either. A fresh
subscription starts silent, like `keyboard` and `workspace` — read
`locked` once for the baseline and apply changes after it — so
subscribing never replays the unsubscribed interval as one change. As
with the query, this is read-only: nothing over IPC locks or unlocks the
session.

Versioning: the subscription is IPC protocol 5 — the `subscribed`,
`output_removed` and `output_restored` tags under the 3 → 4 bump, plus the
`output_changed` tag under 4 → 5 — the keyboard half is protocol 6: the
`keyboard` reply and the `keyboard_changed` tag — the workspace
occupancy event is protocol 7: the `workspaces` tag — and the lock query
and event are protocol 8: the `locked` reply and the `lock_changed` tag. A client that
never sends `subscribe` (or `keyboard`, or `locked`) never receives any of them. An unknown event kind
in a `subscribe` is answered with an ordinary `error` like any unknown
request tag, so an older server meets a newer subscriber with an error,
not a kill.
