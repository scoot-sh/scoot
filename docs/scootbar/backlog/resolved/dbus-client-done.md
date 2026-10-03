---
title: "A shared D-Bus client for the shell"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-02"
---

# A shared D-Bus client

Filed 2026-09-29. Serves **daily-drive**: it is the largest single saving
across the shell if it is small, and the largest cost if it is not.

Notifications (`org.freedesktop.Notifications`), the system tray
(StatusNotifierItem), NetworkManager or iwd, logind and UPower all need D-Bus.
Building it once, in its own small crate, is cheaper than each consumer
pulling `zbus` and its async runtime.

The spike below is this entry's own first task, not a blocker: nothing waits on
it before the entry can start.

## The spike, first

`zbus`, libdbus bindings and a hand-rolled minimal client (auth EXTERNAL,
`Hello`, `RequestName`, method calls, signal match rules). Measure a process that
owns a name and receives a signal: idle RSS, binary size, wakeups. This spike moved
here from [baselines-and-spikes](baselines-and-spikes-done.md) so it is decided when the
first consumer needs it, not before.

## What to do

Take the spike's result. If a hand-rolled client wins, it needs only: the
session and system bus connect, EXTERNAL auth, `Hello`, `RequestName`, method
calls and replies, signal match rules, and a marshaller for the handful of
types the consumers use, all driven from the same `poll(2)` loop (its fd is a
source, not a thread). If `zbus` or libdbus wins by enough on correctness
risk, use it and record the measured cost.

## Guard rails

A hand-rolled bus client parses untrusted bytes from a same-user peer: fuzz
the message parser, cap message and array sizes, and treat a malformed message
as a dropped connection, never a panic. Licence-check anything pulled in.

## Done when

One consumer (start with [scootnotify](../scootnotify.md) or the
[tray](../tray.md)) runs on it and its idle cost is recorded.

## Spike outcome (2026-10-02)

The spike first, the client with the tray. Measured in
[spikes/dbus-client.md](../../spikes/dbus-client.md): a hand-rolled
minimal client beats `zbus` 5 (1,448,032 B, 86 crates, mandatory async
runtime, 4 threads) and libdbus (686,368 B plus `libdbus-1.so.3`) on
every row — 332,456 B, zero dependencies, 1 thread — while all three idle
at zero wakeups. Verdict: hand-rolled, built with the tray (no consumer
exists yet), shape and bounds decided in the record.

## Resolution (2026-10-02, with the [tray](../tray.md), PR #388)

Built where the spike said to: with the tray, its first consumer. The
client is `crates/scootbar/src/dbus/` and adds **no dependency** (the Cargo
feature `tray` is the only switch; `rustix`'s feature set and `Cargo.lock`
are unchanged).

- **`proto.rs`**, `std` only (the fuzz crate compiles it by `#[path]`): the
  framing, a bounds-checked `Reader` and `Writer`, the signature walk, and
  the shape readers the tray uses (`read_item_props`, `read_pixmaps`,
  `read_names`, ...). Bounds: a message past 1 MiB, nesting past 32, a name
  past 128 bytes, a signature past 255, a pixmap side past 256 pixels and
  128 properties an answer are each refused, never a panic.
  Little-endian only: a big-endian message is framed (skipped whole) and
  dropped.
- **`conn.rs`**: auth (EXTERNAL with the empty initial response, per the
  spike's finding 1), `Hello`, calls with a pending table by serial (64
  at most; a call unanswered for 30 s is forgotten when its slot is
  wanted: dbus-broker never times a call out), signals and incoming calls
  as owned events, 64 events a turn, staging and outbox capped at 2 MiB (a
  bus that stops reading drops the connection), and the blocking set-up
  bounded to 2 s in total. The socket is one `poll(2)` source; there is no
  thread and no timer.
- **Tests**: unit tests for the wire, the writer and the bounds; a fixture
  marshalled by sd-bus, which found the one real wire bug in the inherited
  draft (below); the fake bus; and tests against a real `dbus-daemon`
  (`SCOOTBAR_REQUIRE_DBUS_DAEMON` makes their skip a failure; CI sets it).
  The fuzz target `dbus` (the sixth in CI) drives the same functions the tray
  calls.

**What review found in the draft this was built from** (an unreviewed
branch, never merged), each pinned by a test that fails without the fix:

- `array_raw` padded array elements relative to the reader's start, not to
  the message, so an `a(...)` inside a variant that does not start 8-aligned was
  misread: a correct answer with an unknown property holding one was
  refused whole. Pinned by a `GetAll` from sd-bus.
- The fuzz target walked a *copy* of the module's property walk, so the
  fuzzed code was not the shipped code; there is one now (`read_item_props`).
- `scan_names` returned true at the end of every buffer, so every file
  created beside the bus socket cost a connect and a warning.
- `RegisterStatusNotifierItem` announcements from another watcher arrive as
  `service/path`; the host path rejected that form, so a hosting bar never
  showed an app that registered after it started (the fake bus sent the
  bare name, agreeing with the bug).
- A `GetAll` per signal and a shared 64-slot table: an item that never
  answers (dbus-broker has no call timeout) or one that is loud could
  starve every other call. One call in flight per item, a 50 ms floor
  between reads, expiry by age, and a per-service item cap.
- The set-up's timeout was per read (a stopped daemon cost 5 s a step on
  every reconnect), the outbox was unbounded, a bus that dropped the bar at
  once was redialled in a tight loop (41 dials in 2 s in the test), and a
  scroll down sent the same sign as a scroll up.
- `tray` was named in two dead-code allow lists for variants it never
  constructs (the `tray,clock` build failed clippy), and its icon types
  were compiled into builds without the feature.

**Idle cost with its consumer running** is the tray's, published in
[lightest.md](../lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02):
the binary is **+131,072 B** (1,708,768 to 1,839,840 stripped, aarch64),
and with the tray placed and an item on the bus the bar's idle wakeups are
the clock's alone and its RSS is within 0.2 MiB of the bar without it.

**Not built** (the tray's entry lists what it waits on): the system bus
(`connect` takes a path, and only the session address is read), anything
but EXTERNAL, file-descriptor passing, abstract-socket addresses
(`unix:abstract=...` falls through to `$XDG_RUNTIME_DIR/bus`), and
big-endian peers.
