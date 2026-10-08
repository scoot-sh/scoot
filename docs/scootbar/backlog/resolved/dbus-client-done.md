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
[spikes/dbus-client.md](../../../../dev/spikes/scootbar/dbus-client.md): a hand-rolled
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
  `read_names`, ...). Bounds: nesting past 32, a name past the spec's 255
  bytes, a path past 1024, a signature past 255, a pixmap side past 256
  pixels and 128 properties an answer are each refused, never a panic. A
  message past 1 MiB is not refused with the connection: it is skipped
  whole (below), up to the spec's 128 MiB.
  Little-endian only: a big-endian message is framed (skipped whole) and
  dropped.
- **`conn.rs`**: auth (EXTERNAL with the empty initial response, per the
  spike's finding 1), `Hello`, calls with a pending table by serial (64
  at most; a call unanswered for 30 s is forgotten when its slot is
  wanted: no bus times a call out by default, measured on a stock
  `dbus-daemon` for 400 s and on `dbus-broker` for 130 s, so only a client
  library's own timeout would, and this client has none), signals and incoming calls
  as owned events, 64 events a turn, reads stopped at a 1 MiB watermark
  (a flood backs up in the socket and is worked a few pumps a wake; the
  poll is woken with `OUT` meanwhile), a message past 1 MiB skipped whole
  as it arrives with `Event::Dropped` for the call it answered, the
  outbox capped at 2 MiB (a bus that stops reading drops the connection),
  and the blocking set-up bounded to 2 s in total. The socket is one `poll(2)` source; there is no
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
  refused whole. None of the crate's own tests wrote that shape; a
  `GetAll` marshalled by sd-bus did, and pins it.
- The fuzz target walked a *copy* of the module's property walk, so the
  fuzzed code was not the shipped code; there is one now (`read_item_props`).
- `scan_names` returned true at the end of every buffer, so every file
  created beside the bus socket cost a connect and a warning.
- `RegisterStatusNotifierItem` announcements from another watcher arrive as
  `service/path`; the host path rejected that form, so a hosting bar never
  showed an app that registered after it started (the fake bus sent the
  bare name, agreeing with the bug).
- A `GetAll` per signal and a shared 64-slot table: an item that never
  answers (no bus answers for it) or one that is loud could
  starve every other call. One call in flight per item, a 50 ms floor
  between reads, expiry by age, and a per-service item cap.
- The set-up's timeout was per read (a stopped daemon cost 5 s a step on
  every reconnect), the outbox was unbounded, a bus that dropped the bar at
  once was redialled in a tight loop (41 dials in 2 s in the test), and a
  scroll down sent the same sign as a scroll up.
- **Review round 2 (the draft's bounds were the wrong kind).** A valid
  message past 1 MiB (an item's 512 by 512 pixmap: SNI has no way to ask
  for a size, and the comment that said "tens of KiB" was wrong) killed
  the connection, and three such deaths latched the tray off for the
  session; a 2 MiB call to the bar and a signal flood (read without being
  worked, staging passed its cap) did the same. Now skipped and
  watermarked, the latch retries on a timer, and a flood no longer moves a
  megabyte per message (cutting the front of the staging buffer per frame
  was quadratic). Real-daemon tests send each (a 512 by 512 `GetAll`
  answer, a 1.3 MiB call to the bar, 60,000 signals) and assert the other
  item is still shown, the daemon still names the same owner of the
  watcher name, and a newcomer registers.
- **Review round 2 (a hostile item can lose only itself).** A unicast
  `NameOwnerChanged` from any peer removed an item (only the bus's is
  believed now), as did a host-mode announcement from a peer that was not
  the watcher; a well-known name that changed hands kept its old owner, so
  the old owner leaving took the new owner's item; one bad name failed the
  whole `ListNames` reply (and names were capped at 128, not the spec's
  255, and refused a leading dash); a denied `RequestName` was silent; the
  per-service cap counted a service string, not who registered it; paths
  were capped at 1 MiB; unknown header fields refused a message; interface
  names took dashes; abstract session addresses fell through to another
  bus; `array_raw`'s cookie truncated on 32-bit.
- `tray` was named in two dead-code allow lists for variants it never
  constructs (the `tray,clock` build failed clippy), and its icon types
  were compiled into builds without the feature.

**Idle cost with its consumer running** is the tray's, published in
[lightest.md](../lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02):
with the tray placed, an item on the bus and nothing else going on, **zero
idle wakeups** (the clock's two a minute are the only ones in a layout
that has a clock), RSS 4028 kB with the tray alone and 4224 kB with one
item. The binary is **+131,072 B on disk (+7.4%) and +112,496 B of loaded
sections over `main`** (`.text` +96,896 B); the feature built but off is
+65,536 B on disk and +3,312 B loaded, not free.

**Not built** (the tray's entry lists what it waits on): the system bus
(`connect` takes a path, and only the session address is read), anything
but EXTERNAL, file-descriptor passing, abstract-socket addresses
(`unix:abstract=...` falls through to `$XDG_RUNTIME_DIR/bus`), and
big-endian peers.
