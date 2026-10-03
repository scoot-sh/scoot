---
title: "Tray D-Bus client: hardening left over from the #388 review"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Tray D-Bus client: hardening left over from the #388 review

Filed 2026-10-02 from the independent review of the D-Bus client and tray
(PR #388). Serves **daily-drive**: none of these is a live fault, each is a
place a misbehaving peer could cost the bar more than the item it came from.
The review fixed everything it called blocking; these are the non-blocking
remainder, kept so they are not lost.

## The gap

Each was measured or read in `crates/scootbar/src/dbus/conn.rs` and
`crates/scootbar/src/modules/tray/`; none has a failing test yet.

- **No byte budget per pump while discarding.** `read_ready` reads until
  `WouldBlock`, and while a message past 1 MiB is being skipped
  (`apply_discard`) staging stays small, so the loop has no other stop. The
  reviewer streamed 60 MiB messages at the bar for 8 s: the longest single
  pump was 1.5 ms and 11.8 ms was spent in 8 s, so it did not stall. A sender
  that outruns the reader could still hold one turn. A byte budget per pump,
  with the poll woken for the rest, closes it.
- **An oversize reply with more than 64 KiB of header fields is skipped
  unread** (`MAX_OVERSIZE_FIELDS`), so no `Event::Dropped` fires and that
  item's flight and `fetching` flag stay set until a reap, which runs only
  when the flight table is full (30 s). The hostile item loses only itself,
  but a legitimate one that hit this would stay stale.
- **A big-endian oversize header** reads the fields length as little-endian,
  giving a garbage prefix. Harmless today (the message is skipped or its
  header refused), and worth fixing while the framing is open.
- **`hosts` is never pruned** when a host peer disconnects (tiny).
- **`has_room` counts per service or registrant**, so a peer registering 8
  items under another app's service name can crowd that name out. This is the
  pre-existing semantics, and the per-registrant cap bounds it.
- **The fuzz target does not drive the `Conn` skip and discard state
  machine.** The real-daemon and conn-level tests carry that property;
  a fuzz harness over a socket pair would need `conn.rs`'s crate
  dependencies (`print`, `rustix`) in the fuzz crate.

## What to do

Pin each with a test that fails first. Prefer the byte budget (it is a
hang-class bound), then the stuck flight (emit `Dropped` with an unknown token
so the item re-reads), then the rest. Re-measure idle wakeups after the budget:
staged work absent must still request no `OUT`.

## Not in this ticket

Tray menus (the tray ticket), icon-name-only items, and the `Scroll`
direction against a real Qt, GTK or Electron app (needs a real app on
hardware).
