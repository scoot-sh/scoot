---
title: "IPC event for output removed / restored"
status: "resolved"
area: "resolved"
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

## Resolution record

RESOLVED 2026-09-27 (PR #TODO, branch `ipc-output-events`).

**Subscription design (the larger half, no precedent — decided):**

- A `subscribe` *request* on the existing socket, not a second socket.
  `Request::Subscribe { events: Vec<EventKind> }` names the kinds
  (`output` today); the reply is `Response::Subscribed` echoing what was
  asked; afterwards the connection is *dedicated* — it carries events
  only, and any other request on it is refused with an error naming the
  rule. That keeps reply/event ordering trivial (the answer goes out
  synchronously, before any event can exist) and a one-shot client cannot
  wedge itself behind an event it never reads. Requests pipeline on a
  second connection as before.
- Filtering is by kind, server sends every event of the subscribed kinds,
  the client filters/debounces (standby cycles fire routinely — accepted,
  stated on the payload). Adding a kind is additive on the request half
  (an unknown kind is a decode error answered with an ordinary `Error`,
  like any unknown request tag); each new reply tag bumps the version.
- Same socket, same credential story (`0600`, same user) — no new
  permission surface, answering the ticket's socket question.
- Versioning: one bump, `PROTOCOL_VERSION` 3 → 4, for the three new reply
  tags (`subscribed`, `output_removed`, `output_restored`) together. A
  client that never sends `subscribe` never receives any of them.

**Payload:** `OutputRemoved` / `OutputRestored` in `scoot-ipc`'s new
`event` module, in the parent ticket's vocabulary throughout: 0-based
workspace indices, the adopter's previous view as a 0-based index
(snapshotted pre-evict beside the core's relative record), the adopted
block as start + count in the post-removal list, and the origin connector
name. The restore carries the same record as filed plus `moved` (how many
still-open windows actually went back).

**Backpressure (stated + pinned):** emitting never blocks — the subscriber
sockets are non-blocking clones written through the existing `Outbound`
queue. Past the same 1 MiB high-water mark a connection observes, or with
no byte leaving for the same 10-second stall window, the subscription is
dropped and its socket shut down (the peer sees end-of-stream — the
`notify-send` wiring's cue to resubscribe). Output removal never waits
for a subscriber. A disconnected subscriber leaves no record even with no
event to reap it (the accept loop drops it with the connection). Tails
drain on the frame tick, which stays armed while any drains.

**Client:** `scootctl subscribe [EVENT...]` (default `output`) prints the
answer, then one compact JSON object per line per event until killed or
the stream ends (exit 0; re-run to resubscribe). `scoot msg subscribe`
through the same parser.

**Evidence:** 7 harness tests (`outputs/events.rs`: removal + restore
payload without polling with layout cross-checks, hand-move renumbering,
flood-drop + removal-stalls-nothing, stall give-up vs slow-reader kept,
empty-subscribe refusal, drop-forgets) and 6 connection tests
(`connection/tests.rs`: handshake, dedicated refusal, empty keeps
serving, unasked delivery, close drops the record, high-water disconnect
through the real loop); wire pins in `scoot-ipc` (`subscribe` shape,
unknown-kind rejection, both payloads, explicit-null adopter). Full
workspace nextest 2531 green, clippy `-D warnings` + fmt clean, smoke
23-ok green, live `--headless --outputs 2` handshake (`subscribed` + a
`windows` served meanwhile on another connection). No Asahi needed; the
dev-VM vkms rig cannot stage remove/restore (one toggleable connector
only — the last output is never removed), so the two-output headless
harness is the removal proof, as the ticket allowed.

**Review notes for the gate:** the `adopter_active: Some(1)` pin (not 2)
is the adopted block landing at 0..2 and pushing the adopter's own empty
workspace trailing — traced, not snapshotted. The one red in the first
full run was `scootbg`'s backlog-fill test under this shell's `ulimit -n
1024` (EMFILE before EAGAIN); it passes at 65536 and touches no file this
branch changes. Dev VM disk filled once mid-run (`/var/cargo-target`
100%); cleared `debug/incremental` per precedent (7.6 GiB, regenerable).
