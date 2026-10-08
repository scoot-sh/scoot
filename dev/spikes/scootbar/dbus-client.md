# D-Bus client spike: hand-rolled vs `zbus` vs libdbus

Measured 2026-10-02 for [a-shared-d-bus-client](../../../docs/scootbar/backlog/resolved/dbus-client-done.md).
Three throwaway binaries doing the ticket's own job — own a well-known
name, install match rules, receive a signal — one per option, timed and
sized on real buses. The spike crates are removed after; this file is the
record (as with the [config-parser spike](config-parser.md)).

**Outcome: a hand-rolled minimal client wins on every measured row, and it
is the only option that fits the bar's doctrine** (one single-threaded
`poll(2)` loop, no async runtime, no C library outside libc/libm). **It was
built with the tray**, as this record said to, since a client with no
consumer is dead code; the [ticket](../../../docs/scootbar/backlog/resolved/dbus-client-done.md)
records what landed ([What landed](#what-landed), below) and the cost the
bar pays for it is in the [resource ratchet](../../../docs/scootbar/backlog/lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02).

## What was run

- `raw`: std only, zero dependencies (~390 lines with comments): EXTERNAL
  auth, `Hello`, `RequestName`, two `AddMatch` rules, a length-checked
  message reader.
- `zb`: the same job through `zbus` 5.19.0's blocking API
  (`Connection::session`, `request_name`, one match rule, iterate).
- `ldb`: the same job through the `dbus` 0.9 crate (`Connection::session`,
  `request_name`, two match rules, `blocking_pop_message`).

Each binary owned its name (`sh.scoot.SpikeRaw`, `sh.scoot.SpikeZb`,
`sh.scoot.SpikeLdb`), printed its unique name, and sat while the harness
sampled `/proc/PID/status` (VmRSS, threads, context switches) and fd
count, then received one `Ping` signal sent with `dbus-send`. Two buses:

- a private `dbus-daemon --session --fork` 1.16.2 on the dev VM (aarch64),
  disposable, killed after;
- the real session bus on the Asahi box (aarch64, NixOS):
  `dbus-broker-37 --scope user`, read-only — the spike owned test names
  only and sent signals to itself.

Binaries are release builds under the workspace profile (`lto = "fat"`,
`strip = true`). Exact commands, SHAs and raw logs are under [Evidence](#evidence).

## Results

Idle owner holding a name with match rules installed (t0), after a 20–25 s
window with no bus traffic (t1), then one broadcast signal (t2):

| | hand-rolled | `zbus` 5 blocking | libdbus (`dbus` 0.9) |
|---|---|---|---|
| Stripped release binary | **332,456 B** (byte-identical on both boxes) | **1,448,032 B** (+1.1 MB) | **686,368 B** + `libdbus-1.so.3` |
| Crates in the resolve graph, root included | **1** (the root itself; zero dependencies) | **86** | **5** |
| Idle RSS, owner + matches | 1868 kB (daemon) / 2016 kB (broker) | 3028 kB | 2896 kB |
| Threads at idle | **1** | **4** (`zb`, `blocking-1`, `async-io`, `zbus::Connectio…`, all futex-parked) | **1** |
| fds at idle | 5 | 7 | 4 |
| Quiet 20–25 s (t0→t1) | 0 switches, 0 RSS | 0 switches, 0 RSS | 0 switches, 0 RSS |
| One signal (t1→t2) | +2 voluntary switches, RSS flat | +2 voluntary switches, RSS flat | received; delta not isolated |

Cross-box caveat, stated plainly: `zbus` was sized on the dev VM and
libdbus on Asahi, so their RSS rows carry the ~150 kB environment gap the
hand-rolled binary shows between the two boxes (1868 vs 2016 kB). The
ordering survives any reading of that gap: hand-rolled is ~1.1 MB smaller
on disk and ~1 MB lighter resident than `zbus`, with none of its threads.

License check (the ticket requires it): `zbus` 5.19.0 is MIT and a scan of
all 86 resolve-graph crates' `license` fields shows no binding copyleft:
mostly MIT/Apache-2.0/ISC/BSD, plus unicode-ident (Unicode-3.0), one
Unlicense-or-MIT crate, and r-efi (LGPL-2.1-or-later as one option among
MIT/Apache-2.0) — every outlier ships a permissive option the bar can
take, so the conclusion holds with a true enumeration. The `dbus` crate is Apache-2.0/MIT, but it links the
system's `libdbus-1` — a C library outside the bar's current closure
(libc, libm and libgcc_s only, per the network module's measurement), whose
license must be checked against whichever copy the closure carries when a
consumer lands (the NixOS lib output ships no COPYING file; checked
2026-10-02).

## Reading

- **Idle behavior ties**: all three sit at zero wakeups with no traffic and
  cost ~2 context switches per delivered signal. The bus fd is waitable in
  `poll(2)` in every option, so none of them threatens the ratchet's
  two-wakeups-a-minute target by itself.
- **Everything else decides.** `zbus` cannot be used without an async
  runtime at all: with `default-features = false` the crate refuses to
  compile (`Either "async-io" (default) or "tokio" must be enabled`), and
  its blocking API is a wrapper over an async message stream (`block_on`
  per message) with a parked 3-thread pool. That is a second runtime in a
  bar whose doctrine is one single-threaded loop — rejected on fit before
  the megabytes. libdbus fits the loop (its fd dispatches by hand) but
  pulls a C shared object into a closure that currently has none, for a
  client the bar can write in ~400 lines of dependency-free Rust reusing
  the volume/network playbook (bounded reader, faked bus in tests, fuzz
  target + committed corpus, stable corpus replay).
- **Build it later, with the tray.** The spike's verdict on *shape* is
  final; its verdict on *timing* is that nothing should land until the
  first consumer does. A client with no consumer is dead code the soak and
  the ratchet cannot exercise.

## Wire findings (checked against the live daemons, for the implementer)

1. **Auth: send EXTERNAL with an empty initial response.** `AUTH
   EXTERNAL\r\n`, wait for `DATA`, reply `DATA\r\n`, wait for `OK`. On
   dbus-daemon 1.16.2, sending the hex uid explicitly (`AUTH EXTERNAL
   3e8`, or a later `DATA 3e8` for uid 1000, the connecting user) is
   answered `REJECTED`, while the empty form — the exact bytes sd-bus
   sends, confirmed by tracing `busctl` — authenticates. Same code works
   against dbus-broker. The client sends the empty form everywhere.
2. **The header array's length word is bytes 12–16; its elements start at
   16.** A reader that takes the length from 16 parses garbage, and a
   sender that appends a second length word is disconnected by the daemon
   without a reply. Both mistakes were made and caught in the spike.
3. **Replies must be demultiplexed by serial — "send, read one" breaks.**
   Owning a name emits server signals (`NameAcquired`, and
   `NameOwnerChanged` once matched) that arrive *between* a call and its
   reply, so the client needs a pending-call table keyed by serial plus
   signal dispatch, even before any subscription exists. The spike loops
   until the reply serial arrives.
4. **The daemon validates the wire, so the threat model is valid-but-hostile
   shapes, not malformed bytes.** A same-user peer cannot smuggle a
   malformed message through either daemon; it can send absurd-but-valid
   ones (deep nesting, huge arrays, megabyte pixmaps). The parser's bounds
   and its fuzz target should therefore mutate valid shapes (nesting,
   lengths, array counts), not just random bytes.

## The shape, decided

- One connection per bar process, multiplexed: pending calls by serial,
  consumer callbacks by match rule, `NameOwnerChanged` tracked centrally
  (tray re-acquisition, MPRIS player appear/vanish, BlueZ adapter loss all
  hang off it). The socket fd is a poll-loop source like the netlink
  sockets — no thread, no timer.
- Marshaller scope is the type set the four consumers need, nothing more:
  all basic types, plus `ARRAY`, `STRUCT`, `VARIANT` and `DICT_ENTRY`
  (tray pixmaps `a(iiay)`, `PropertiesChanged` `sa{sv}as`, BlueZ
  `a{oa{sa{sv}}}`, Notify hints `a{sv}`). Strings cut like view text (the
  256-byte rule), names bounded like the volume module's 128-byte rule.
- Bounds discipline follows the siblings: a message cap set from the
  largest legitimate payload (tray pixmaps at requested device pixels are
  tens of KiB; menu layouts are the unbounded one — 1 MiB is the starting
  point for the tray entry to confirm, not the spec's 128 MB), an explicit
  nesting-depth limit, every refusal a dropped message never a panic, a
  fuzz target plus committed corpus plus stable replay as in
  `modules/volume/fuzz.rs` and `modules/network/fuzz.rs`, and
  feature-gated code that compiles in the lean builds (the volume lane
  broke those twice).
- Event-loop integration points for the tray entry: fd source, match-rule
  multiplexing, name-owner tracking, and the pending-call table — the
  spike proved all four against both daemons.

## Evidence

- Worktree at `2cb658e77` (`origin/main` 2026-10-02, rebased from `8c7d9795`)
  plus this record and the ticket note; no `.rs` touched (`git status`
  shows two `.md` files).
- Dev VM (aarch64, rustc 1.97.1): private `dbus-daemon --session --fork`
  1.16.2, `CARGO_TARGET_DIR=/tmp/dbus-spike-target`, release profile as
  above. `raw` 332,456 B; `zb` 1,448,032 B (`--features zbus-bin`, the
  spike crate's own flag enabling its optional `zbus` dep with
  `features = ["blocking-api"]` and default features on).
  `cargo tree` resolve totals including the root: 1 (itself) vs 86. Quiet windows: `raw` 11→11
  voluntary switches, RSS 1868→1868 kB (20 s); `zb` 12→12, 3028→3028 kB
  (20 s). One `Ping`: both +2 voluntary switches, RSS flat.
- Asahi M2 (aarch64, NixOS, `nix develop` from `~/code/scoot` for the
  linker): `dbus-broker-37 --scope user` at `/run/user/1000/bus`.
  `raw` 332,456 B, RSS 2016 kB at idle; `ldb` 686,368 B +
  `libdbus-1.so.3`, RSS 2896 kB, 1 thread, 4 fds, 22 s quiet with zero
  delta. `Ping` received on both.
- Not measured: per-call latency beyond the switch counts above (both
  complete in one round trip; the bar's frame budget is unaffected either
  way), `zbus`/`libdbus` on the broker bus, suspend/resume with the bus
  down (the tray entry owns that test: reconnect + re-acquire + re-match).

## What landed

The shape below held: one connection, a poll-loop fd, a pending-call table
by serial, central `NameOwnerChanged` tracking (the tray's items vanish with
their owners, and the watcher name is re-taken when it is lost), a marshaller
for exactly the type set the tray needs (`a(iiay)`, `a{sv}`, `(sa(iiay)ss)`,
`as`, the basic types and variants). Where it moved:

- **Wire finding 4 stands as written** (the daemons validate the wire, so
  the threat is valid-but-hostile shapes, not malformed bytes). The one
  wire bug the build found was the reader's, not the protocol's: an
  8-aligned array inside a variant that starts off an 8-byte boundary, a
  shape none of the crate's own tests wrote, so the reader padded it
  relative to the variant instead of the message. A message marshalled by
  sd-bus held that shape (`src/dbus/fixtures/`, which says how it was
  captured), and now the test suite does.
- **No bus times a call out by default.** Measured with a peer that owns a
  name and never replies (`bench/m6-tray-vm/logs/call-timeouts.txt`): a
  stock dbus-daemon 1.16.2 `session.conf` returned nothing in 400 s (its
  `reply_timeout` limit, which a config can set, did answer at the 5 s
  configured in the test), and the VM's dbus-broker nothing in 130 s; the
  25 s everyone remembers is libdbus's and sd-bus's own client timeout. A
  client with none, as this one, must forget its own calls, so the pending
  table expires them by age when its slots are wanted (no timer: an idle
  bar stays at zero wakeups).
- **The 1 MiB bound held, but not as a refusal.** The record's premise
  (pixmaps at requested device pixels are tens of KiB) was wrong: SNI has
  no way to request a size, so an item may send a 512 by 512 pixmap, which
  is a valid message just over 1 MiB, and the first build killed the
  connection on it (and the tray stayed off for the session). A message
  past the cap, up to the spec's 128 MiB, is now skipped whole as it
  arrives, a reply says which call lost its answer, and only a header that
  is no message ends the connection; a flood is read only up to a
  watermark. Menu layouts, the other unbounded one, wait on the tray's own
  DBusMenu client (popups exist).
- **The set-up is the one blocking part** (auth and `Hello`, three round
  trips): bounded to 2 s *in total*, not per read.
- Costs, recorded where the ratchet keeps them, not repeated here.

## One connection per consumer: decided (2026-10-03)

The record's shape said one multiplexed connection per bar process, with
pending calls by serial, consumer callbacks by match rule, and central
`NameOwnerChanged` tracking. What landed instead is one connection per
consumer (the tray's, then the media module's, now both on the same
`link` lifecycle), and that is the decision, not a halfway state:

- **Measured cost of a second connection is one fd and a few kilobytes.**
  The tray's rows and the media module's rows each show 8 fds alone (one
  of them the bus socket) and zero idle wakeups with no bus, a bus with
  nothing on it, and a bus with items or players. There is no pressure to
  share on any measured row.
- **Failure isolation.** A poison message that ends the connection (a
  header that is no message, a bus that stops reading past the outbox
  cap) drops one consumer's session — its pending calls, its tracked
  names — and the other never notices. A shared connection would take
  both sessions down together, and couple their retry states.
- **The match rules do not union.** The media module's zero-wakeup rows
  depend on the bus filtering to its two narrow rules (MPRIS namespace
  owner changes, one object's `PropertiesChanged`); the tray needs five
  broad ones. A shared connection would work every one of the tray's
  signals through the media module's pump too — parsing and discarding
  another consumer's traffic on every wake — for no measured gain.
- **What sharing would need** (pending table keyed by consumer as well
  as serial, match-rule reference counting across consumers, one owner
  map and one re-acquisition policy for two different name sets) is
  shared mutable state between modules the project does not otherwise
  have, built for a third consumer (notifications, BlueZ) to complicate
  further.

Revisit if a measured row says otherwise (fds, memory, wakeups under a
real multi-consumer load); until then a consumer is a connection, and
the multiplexing the record imagined stays unbuilt.
