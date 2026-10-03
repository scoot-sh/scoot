---
title: "Tray D-Bus client: hardening left over from the #388 review"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
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
- **Method replies are matched by serial only** (`Conn::dispatch`,
  `conn.rs` ~L647-668): `Pending` records no destination, and serials are
  small and sequential. A peer that can guess one and gets an unsolicited
  reply delivered could forge a `GetNameOwner`, `GetAll` or `ListNames`
  answer (the media review showed what that buys: re-pointing a held player
  name at the forger, or setting a player's displayed state). A forger can
  already claim its own name, so the extra power is small. Not tested:
  whether `dbus-daemon` delivers an unsolicited reply to a non-caller at all
  (the reviewer read the code only). Check that first; if it does, record
  the callee's destination (or its unique name once known) and refuse a
  reply from anyone else.
- **`Conn::dispatch` allocates about six times per event**, which breaks the
  per-event allocation rule (about 13 µs per `Position` signal at 500 a
  second, measured by the media implementer). Not a live fault; worth
  reusing buffers.
- **`runtime_dir` exists three times** (`conn.rs`, the tray, the volume
  module); the media work made the `conn.rs` one public.
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

## Resolution (2026-10-03, PR #TBD)

All nine items landed, each with a test that failed first (failures
quoted below are the pre-fix runs on the dev VM, aarch64):

- **Byte budget.** `Conn::read_ready` reads at most 256 KiB more per pump
  while discarding an over-cap message, with `backlog` set so the poll
  returns at once for the rest. Test
  (`discarding_a_flood_is_bounded_a_pump_at_a_time`, 60 MiB stream):
  failed first with "25745728 bytes in one pump"; after, the stream
  drains in 1868 pumps, longest 1.70 ms, and the idle state asks for no
  `OUT` (`has_staged_work`/`want_write` false).
- **Stuck flight.** A skipped reply no call can be named emits
  `Event::Dropped` with `DROPPED_UNKNOWN` (only when the message is a
  reply to a call still waiting; calls, signals and strays stay silent),
  and the tray and media modules reap by age and let every waiting item
  ask again at its next signal. Tests (over-cap fields past 64 KiB,
  both consumers' release-everything tests): failed first with no event;
  pass after.
- **Big-endian header.** The fields length is read in the message's own
  byte order. Test (byte-swapped over-cap reply): failed first with no
  event; passes after, as an unknown drop (the parser stays
  little-endian-only).
- **`hosts`.** Pruned when the owner name goes away. Test: failed first
  (nothing ever removed a host entry); passes after.
- **`has_room`.** Decided and documented on the function: service OR
  registrant, kept — by AND one peer could hold 8 under every name —
  with a pinning test (characterization: passes before and after).
- **Serial-only replies: REAL, fixed properly.** First run as a probe:
  `dbus-daemon` 1.16.2 delivers an unsolicited `METHOD_RETURN`/`ERROR`
  from a non-callee peer to the destination (the victim took the forged
  body as its reply, twice: peer call and bus call), while `dbus-broker`
  37 does not (the forgery never arrived; the real answer did). So
  `Pending` records the callee, and `dispatch` (plus the over-cap and
  set-up paths) refuses a reply whose sender is not it whenever the
  callee is known — the bus's own calls, or a unique name's; well-known
  names and sender-less scripted peers are accepted as before, and a
  refused reply leaves the flight for the real answer. Tests: both
  failed first (forged body arrived / `read_names` refused the forged
  body); pass after.
- **Allocations.** Measured through the counting allocator against a
  real daemon: six small allocations per signal (sender, path,
  interface, member, signature, body) plus the events vec's first growth,
  zero for an idle pump — pinned by
  `a_signal_costs_six_small_allocations` (stable across reruns). All six
  are inherent owned data over a reused buffer on a rare path, so
  nothing was taken; the test is the bound.
- **`runtime_dir`.** One helper (`conn::runtime_dir`); the tray's and
  the volume module's copies (identical bodies) are gone. Volume tests
  pass unchanged in shape.
- **Fuzz.** Not driven: `conn.rs` needs `crate::print` and `rustix`,
  and compiling it against stubs would fuzz adapted code, not the
  shipped code. Said so in `fuzz.rs`; the conn-level socketpair tests
  above carry the property instead. The `dbus` target ran 6,410,554
  runs in 301 s with no finding.

Idle wakeups after the budget: re-measured with the link work (same
harness, same box): zero in all eight tray-alone rows, branch and
`main` alike — see the ratchet's [new section](../backlog/lightest.md).
