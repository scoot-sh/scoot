---
title: "Move the tray's bus lifecycle onto dbus::link, and decide on one shared connection"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
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
