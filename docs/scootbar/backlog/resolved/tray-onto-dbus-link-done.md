---
title: "Move the tray's bus lifecycle onto dbus::link, and decide on one shared connection"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# Move the tray's bus lifecycle onto dbus::link, and decide on one shared connection

Filed 2026-10-03, by the [media module](resolved/media-module-done.md)'s PR
(#393).
Serves **daily-drive**: no behavior changes, one less copy of a state
machine that has to be right.

## The gap

The same lifecycle exists twice. `crates/scootbar/src/modules/tray/mod.rs`
(`Bus`, `wait`, `drop_live`, `connect`, `arm_retry`, `on_retry`,
`on_notify`, `on_bus`, `scan_names`, `runtime_dir`; about 250 lines) and
`crates/scootbar/src/dbus/link.rs` (the media module's, written from it
with the consumer's state made a type parameter) both wait on the bus
socket's directory, dial when it appears, drop the whole session when the
connection dies and redial once, and leave a bus that keeps dropping the bar
alone for 30 seconds. A fix to one (a wake that missed the socket, a retry
that spun) must be made in both, and the tray's tests of it
(`modules/tray/tests.rs`) do not cover the media module's copy nor the
reverse; `dbus/link/tests.rs` covers `link` against a real daemon and a bus
that takes the bar in and drops it.

Each module also holds its own connection: with both placed the bar has two
sockets to the session bus, two `Hello`s and two sets of match rules, where
the [spike](../spikes/dbus-client.md) imagined one multiplexed connection
(pending calls by serial, consumer callbacks by match rule,
`NameOwnerChanged` tracked centrally). Measured cost is one fd and a few
kilobytes; the cost that matters is that a third consumer (notifications,
BlueZ) makes it three.

## What to do

1. Port the tray onto `Link<Live>` (`Live` already owns its `Conn`: give it
   `impl Session`), delete its copy, and keep `modules/tray/tests.rs` as the
   proof that nothing observable moved. The tray's `RETRY_AFTER_QUICK_DEATHS`
   and `QUICK_DEATH` are `link.rs`'s now.
2. Decide whether one shared connection is worth it: it needs the pending
   table keyed by consumer, match-rule reference counting, and one owner
   map. If not, say so in the spike record and drop the sentence from
   `dbus/mod.rs`.

## Not in this ticket

The tray D-Bus client's hardening left over from the #388 review (a byte
budget per pump while discarding an oversize message, an oversize reply
whose header fields are past 64 KiB firing no `Dropped`, `hosts` never
pruned), which is filed on the `docs/tray-review-followups` branch and
applies to both consumers alike: the media module does not depend on any
of it (it tracks the age of its own call and forgets it past 30 seconds).

## Resolution (2026-10-03, PR #TBD)

- **Ported.** The tray is a `Link<Live>` (`impl Session` on the
  watcher's `Live`; the refresh timer past the link's sources; a dropped
  session reported changed only when icons left with it). Its ~250-line
  copy (bus, wait, drop_live, connect, arm_retry, on_retry, on_notify,
  on_bus, scan, retry constants) is deleted: net -329 lines in the PR.
  `modules/tray/tests.rs` passes with explained edits only: `BusAddr` is
  the link's `Addr` (`Stream` now watches a fixed nonexistent path, so a
  dropped test bus can no longer redial the real session bus — the fix
  the ticket asked for, by construction), the directory-scan test is
  deleted (the scan lives in `link`, whose own tests pin it), and the two
  quick-death tests follow the link's 300 ms test retry instead of the
  old copy's 2 s. The tray's `RETRY_AFTER_QUICK_DEATHS` and `QUICK_DEATH`
  are `link.rs`'s now.
- **One shared connection: decided no.** Recorded in the
  [spike](../spikes/dbus-client.md#one-connection-per-consumer-decided-2026-10-03)
  and the `dbus/mod.rs` sentence now says so: a second connection costs
  one fd and a few kilobytes (measured), a poison message drops one
  consumer's session instead of both, and the match rules do not union
  (the media module's zero-wakeup rows need its two narrow rules; the
  tray needs five broad ones).
- **`init` blocking: kept, bounded.** The dial (auth and `Hello`) is
  bounded to 2 s in total by the set-up timeout; avoiding it entirely
  needs a ready-driven set-up across both consumers and their tests, so
  it stays as it is, identical for the tray and the media module.
- **Behavior and idle cost identical by measurement**, not by
  assertion: the tray-alone rows read zero wakeups with no bus, a bus
  with no items, and one and eight items, RSS level with the same
  tree's `main` in every row (+4, -4, +0, +20 kB — none resolved by one
  run), and the binary is +2,144 B of `.text` with no byte on disk.
  Full table in the ratchet's [new section](../backlog/lightest.md);
  no row regresses, none waived.
