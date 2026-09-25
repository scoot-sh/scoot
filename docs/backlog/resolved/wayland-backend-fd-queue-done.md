---
title: "wayland-backend kept a client's received fds in an unbounded queue: one connection could fill scoot's fd table — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Unbounded per-connection received-fd queue (wayland-backend) — RESOLVED

RESOLVED 2026-09-25 (PR #241) by route 2 below, together with
[raising scoot's fd limit](./raise-nofile-limit-done.md): scoot builds
against a scoot-sh fork of wayland-backend (`docs/forks.md`), pinned through
the root `Cargo.toml`'s `[patch.crates-io]` at `70f81e00` (the 0.3.17
release `72f7fe0d` plus two server-side commits), and raises its own soft
`RLIMIT_NOFILE` at startup. Nothing was filed upstream.

## What changed

- **The fork.** In the server's `Client::next_request`, when every complete
  request in the buffer has been parsed and before the next read: a client
  with more than its cap of received fds still queued is sent
  `wl_display.error` `invalid_method` ("too many file descriptors queued
  (more than N)"), the queued fds are closed and the client is
  disconnected (`a39311b8`). The cap is one eighth of the soft
  `RLIMIT_NOFILE` read when the client is created, clamped to 128..=1024
  (`70f81e00`): 1024, libwayland-server's default bound, on the table scoot
  raises to; 128 where the hard limit is 1024.
- **The raise** (`crates/scoot/src/compositor/nofile.rs`): the soft limit
  is set to min(hard limit, 65536) at startup, lowering one that started
  higher (Docker before 25 starts containers at 1048576:1048576); every child
  `State::spawn` starts gets the original back. See its own record.
- **The pin.** `Cargo.lock` moves exactly two entries, `wayland-backend`
  and `wayland-sys`, to the fork's git source (`wayland-sys` byte-identical
  to crates.io 0.31.11); `flake.nix` has one `outputHashes` entry.
- **Tests in scoot** that fail if the patch is dropped and pin the numbers
  the arithmetic uses, at whatever limit the test process runs with
  (`fd_pressure/tests/backend_queue.rs`: exactly the cap kept, one more
  disconnected with the fork's error, the parked fds closed, one read adds
  at most 30), and the legitimate shapes
  (`fd_pressure/tests/backend_queue_client.rs`: under backpressure the cap
  is served and one more disconnected; a pure-Rust client's largest
  one-flush batch, 1036, served and 1037 disconnected).
  `gamma_rapid_sets_stay_alive` is back to its original 200-unflushed shape.
- **fd pressure's arithmetic** (`fd_pressure.rs`) adds the queue on both
  tables: 1674 per connection at every bound against 65408 on the raised
  table; 748 (778 inside a read) against 896 on a 1024 one.

## How it got here: PR #241's first round, and why it changed

The first version of this PR pinned `a39311b8` alone, a fixed cap of 128,
sized against the default 1024-fd table. Its record said the only
legitimate clients it could disconnect were pure-Rust ones batching more
than 140 fd-carrying requests per flush, and that libwayland clients could
not reach it. **Review of PR #241 showed that was false:** libwayland 1.26
keeps queueing after its flush hits `EAGAIN` (buffers from
`wl_display_connect` are unbounded), then sends its fds 28 per `sendmsg`
ahead of the requests, so a stock libwayland client stalled behind a busy
compositor reached the cap. Measured by the reviewer on the dev VM
(`~/review241/bp/`): the compositor stopped, the socket filled, N
`create_pool`+destroy pairs queued, the compositor resumed; the fixed-128
fork killed the client at N of 140 and above, `main` served 160 and 600.
"Flush every 140" could not help under backpressure, and eight idle
connections still got past the fixed cap to fill the table. The
coordinator's decision was the raise plus an adaptive cap, in this PR. The
first round's own record is kept below, under "First round (superseded)".

## The accept-storm freeze (round 3)

Re-review of the raise found that on a 65536-entry table the pressure
observation, a readdir of `/proc/self/fd` linear in open fds (~7 ms at
60000), ran once per accepted connection, and the Wayland accept callback
drains up to 4096 connections at once. The reviewer's `storm.sh` on the dev
VM: 58 connections parking 1008 fds each (58540 open, all under the cap and
connected), then 4000 connections in 13 ms, froze scoot for **35.6 s** for
an ordinary round-trip client (6.9 s at ~12000 parked; 0.96 s with nothing
parked; `main` on its 1024 table: 0.36 s).

- **The fix** (`fd_pressure::table`): one per-thread cached reading, reused
  for max(1 ms, 20 x what it cost), so observing costs at most ~5% of loop
  time on any table; every admitted Wayland or IPC connection counts +1
  against it (`note_opened`), so a burst is still judged connection by
  connection. `pressure_refusal` and the ledger's arrival guard read the
  same cache. A cheaper exhaustion signal was considered and rejected:
  `fcntl(F_DUPFD_CLOEXEC, line)` only says whether some fd at or above the
  line is free, which with lowest-first allocation and holes below the line
  is not the count the reserve is defined on.
- **Measured** (`runs/storm-b596906.txt`, `runs/fd-storm-script-*.txt`,
  binary `bin/scoot-gauge-b596906-debug`): the same storm, 58 parkers and
  4000 connections, now gives a worst round-trip wait of **101 ms** (110 ms
  through the committed `scripts/fd-storm/run.sh`); the uncached `2c18a93`
  through that script: **36.1 s**. With nothing parked: 109 ms; `main`:
  361 ms. Crossing the line (63 parkers, 63585 open, then 4000): scoot
  stopped at 65406, just under the 65408 line, shed 2179 connections, and
  the worst wait was 72.5 ms.
- **Tests** (`fd_pressure/tests/gauge.rs`): a 300-connection burst through
  the real listening socket makes no observation and counts all 300; a
  burst ten fds from the line admits exactly the eleven the reserve allows
  and sheds the rest; 1001 back-to-back readings make one to three
  observations. With caching off, five of the six fail
  (`runs/failfirst-gauge-uncached.txt`).
- **Also in round 3:** the soft limit is clamped down to 65536 as well as
  up (checked live: started at 200000:524288, scoot ran at 65536 and its
  children at 200000, `runs/limits-soft200k-gauge-b596906.out`); the
  child's `setrlimit` can no longer fail a spawn. The cap stays 65536, now
  justified by the storm figures above rather than by the uncached cost.

## The stale-reading fill (round 4)

Final re-review found the cached reading could go stale-low: within its
~140 ms lifetime a client past its grace opened far more than the 128-fd
reserve through guarded paths, and the reading waved them through. The
reviewer's `stale.sh` (63 connections parking 1024 each, 64593 open, one
accept to force a fresh reading, then two clients each keeping 512 shm pool
fds): on `383c6b9` a holder was disconnected by `ECONNRESET` and scoot
installed fds up to 65535 before `recvmsg` truncated; the uncached `40fd41d`
refused one holder with a protocol error.

- **Fix** (`46ccbb8`): `client_fds::record_arrival` counts every admitted
  pool, plane (at its admitted weight, renderer copies included) and
  timeline fd against the reading, and each admitted acquire wait counts
  its eventfd; the reading's lifetime is capped at 250 ms, so a preempted
  readdir cannot stretch it. What stays uncounted within a lifetime is
  listed in `fd_pressure::table`'s doc (scoot's own fds beyond admission
  weights, and wayland-backend's queue, as for the uncached observation).
- **Measured** (`runs/stale-46ccbb8.txt`): the same run on `46ccbb8` refuses
  the second holder with a protocol error on `wl_shm` (the client sees
  `EPROTO`) at a peak of 65106 open, under the 65408 line; `383c6b9` in the
  same run: `ECONNRESET`. The storm stays fixed: worst wait 122.8 ms
  (`runs/fd-storm-script-46ccbb8.txt`).
- **Tests:** `client_fds/tests/shm.rs`
  `a_past_grace_client_is_refused_at_the_line_on_a_stale_reading` (a reading
  pinned ten fds short of the line and never refreshed; the past-grace
  client is refused within the line plus `SWEEP_MARGIN`; without the
  counting it survives, `runs/failfirst-stale-reading-uncounted.txt`), and
  `gauge.rs` `an_expired_reading_is_taken_again`.

## What it costs a legitimate client now

- **With the raise (hard limit 8192 or more): nothing libwayland-server
  would refuse.** The cap is libwayland-server's default 1024. A
  libwayland client stalled behind a stopped scoot is served at 160, 600
  and 1000 queued fd-carrying requests (`runs/bprun-160-600.txt`); a
  pure-Rust client's one flush is served up to 1036.
- **Hard limit 1024 (a container): the first round's cost remains.** The
  cap stays 128, logged at startup; the same libwayland probe is served at
  120 and disconnected at 160 (`runs/container-bprun-raise-2c18a93.out`).
  Documented in `protocols.md` and the CHANGELOG with the fix (raise the
  hard limit).

## What is left

- **Connection multiplication**, now at the raised table: each idle
  connection can park 1024 with no objects, which neither the ledger nor
  the grace sees. 63 such connections held 64593 fds with everyone served;
  64 filled the 65536 table (newcomers dropped, `scootctl` reset). On a
  1024-fd table it is 7 and 8 connections, as before. Recorded on
  [`pressure-many-light-connections`](../core/pressure-many-light-connections.md),
  which stays open.
- **Observing a full table** costs one readdir of `/proc/self/fd`, ~7-8 ms
  at 65000 open fds (`runs/readdir-cost.txt`), at most once per ~140 ms: the
  reading is cached (see "The accept-storm freeze" below).
- **The 1024-table drain-window transient** (1034, reasoned in
  `fd_pressure.rs`), only where the hard limit keeps the table at 1024.
- **The parked-syncobj over-count** (`client_fds.rs`) is bounded by the cap,
  not removed. **Outgoing fds** are unchanged in the fork, reasoned only.

## Evidence (final)

Dev VM (kernel 6.18.50, hard `RLIMIT_NOFILE` 524288, soft 1024 in the ssh
shell), debug builds, all under `~/evidence/fdq/`. Final binary: commit
`2c18a93` (`bin/scoot-raise-2c18a93-debug`, sha256 `3075f728…`); later
commits change docs, and one removes the XWayland limit toggle, which the
default build does not compile. For comparison, the first round's
fixed-128 build `8b01249` and `main` `173029d`.

- **The reviewer's libwayland backpressure probe** (`bprun.sh`,
  `runs/bprun-160-600.txt`): new build served at N=160 (`roundtrip=162`),
  600 and 1000, scoot back to 18 fds; the fixed-128 build disconnected both
  160 and 600 with "more than 128".
- **The PR #236 attack probes**: 40 messages of 28 fds is disconnected at
  1036 with "more than 1024", scoot back to 18, `wayland-info` and
  `scootctl` served (`runs/attack-raise-2c18a93-N40.out`); the reviewer's
  original 35 x 28 (980, under the cap) stays parked at 999 fds with
  `wayland-info` (38 globals) and `scootctl` served on the raised table
  (`runs/orig-probe-raise-2c18a93.out`), where `main` shed both.
- **Many idle connections** parking 1024 each
  (`runs/many-connections-raise-2c18a93.out`): 8 connections 8218 fds, 62
  63568, 63 64593, all served; 64 filled the table (65536).
- **Limits** (`runs/limits-raise-2c18a93.out`): scoot soft 65536 / hard
  524288; `foot` started as the session command and by IPC `spawn`, and
  their shells, soft 1024 / hard 524288. Startup log: `raised the fd limit
  ... from=1024 to=65536 hard=524288 unclaimed_fd_cap=1024`.
- **Container** (`prlimit --nofile=1024:1024`,
  `runs/limits-container-raise-2c18a93.out`,
  `runs/container-attack-raise-2c18a93.out`): log line `fd limit not
  raised ... unclaimed_fd_cap=128`; 140 parked disconnected, 112 held;
  children 1024/1024.
- **XWayland** (`xwayland` build at `2c18a93`, `runs/xwayland-limits.out`):
  scoot 65536, the Xwayland server it started 524288. That build still put
  1024 back around the spawn, so the server started at 1024 and raised
  itself to its hard limit; `7ce06df` dropped the toggle for that reason
  (see the raise record).
- **scoot's tests, fail-first** at the new tests:
  `runs/failfirst-tests-no-patch-0.3.17.txt` (no patch: the four
  disconnect tests fail), `runs/failfirst-tests-old-fork-a39311b.txt`
  (fixed 128: the serve tests and `gamma_rapid_sets_stay_alive` fail),
  `runs/failfirst-child-limit-restore-removed.txt` (the child reads 65536
  without the restore).
- **The fork's own tests** on Linux at `70f81e00`
  (`runs/fork-adaptive-cargo-test.txt`): 98 + 1 + 2 passed, including the
  new cap-formula test.

## First round (superseded)

Kept as written at `eba798b`; its libwayland claims are wrong (above).

### What changed (first round)

- **The fork's one commit.** In the server's `Client::next_request`, when
  every complete request in the buffer has been parsed and more bytes are
  needed, and *before* reading them: if more than 128 received fds are
  still queued, the client is sent `wl_display.error` `invalid_method`
  ("too many file descriptors queued (more than 128)"), the queued fds are
  closed and the client is disconnected. Checked before the read so that
  fds the next bytes will claim are not counted against it; libwayland
  clients send each fd with its request and never come near it.
- **The pin.** `Cargo.lock` changes in exactly two entries,
  `wayland-backend` and `wayland-sys`, both to the fork's git source (the
  fork leaves `wayland-sys` byte-identical to crates.io 0.31.11, and its
  `wayland-backend` source differs from crates.io 0.3.17 only by that
  commit: diffed). `flake.nix` gains one `outputHashes` entry.
- **A regression net in scoot** (`fd_pressure/tests/backend_queue.rs`)
  that fails if a repin, `cargo update` or rebase ever drops the patch, and
  pins the numbers scoot's arithmetic uses: exactly 128 parked fds are kept
  and the 129th disconnects with the fork's error, the parked fds close,
  one read adds at most 30 (the receive buffer rustix sizes for 28, padded
  by `cmsghdr` alignment), and a flush of 140 fd-carrying requests from a
  `wayland-client` on its pure-Rust backend is served while 141 is
  disconnected.
- **fd pressure's arithmetic** (`fd_pressure.rs`) adds the queue: 128 at
  rest, 158 for a moment inside the read past the bound.

### What it costs a legitimate client (a finding, not hidden)

The check counts fds a client has sent ahead of their requests. A client
on `wayland-client`'s pure-Rust backend (the rs backend, its default)
grows its outgoing buffer without limit (`max_buffer_size: None` in
`client_impl`) and, on flush, sends every fd past the last 28 ahead of the
bytes, 28 per `sendmsg` with one byte each. So such a client that queues
**more than 140 fd-carrying requests between flushes** (pools, planes,
timelines, gamma ramps, selection receives) has 140 queued before the
final write and is disconnected. 0.3.17 served it. Measured on the dev VM
with a standalone pure-Rust client against the real binary: 28, 100, 128
and 140 served, 141 and 200 disconnected with the fork's error (`main`
served all six). One scoot test hit it: `gamma_rapid_sets_stay_alive`
queued 200 `set_gamma` before one round trip; it now flushes every 64
ramps (still back to back, which is what it tests).

Who can hit it, as far as was checked (Cargo manifests on 2026-09-24, no
client measured beyond the probe):

- **Not** libwayland clients (C/C++, GTK, Qt, foot, Firefox, mpv...):
  libwayland flushes before its 29th pending fd, so its fds never run more
  than one `sendmsg` ahead.
- **Not** Rust clients whose dependency tree enables wayland-backend's
  `client_system` feature, which switches every `wayland-client` in the
  program to libwayland: winit 0.30.x and master (`winit-wayland`) force
  it, so iced, egui/eframe, Bevy, Alacritty-class apps are on libwayland;
  softbuffer enables it too; and any client rendering with EGL or Vulkan
  needs a libwayland `wl_display*` to hand the driver, which only that
  backend provides.
- **Exposed:** pure-Rust clients on the rs backend, e.g. ones built
  directly on Smithay's client toolkit (its `system` feature is opt-in)
  drawing into shm, when they queue more than 140 fd-carrying requests
  before returning to their event loop. No such client was found or
  measured; typical ones create a few buffers per surface.

This is a new client disconnect for a legitimate (if unusual) shape, so it
is a harm-rule decision (`CLAUDE.md`, "Never defer a user-facing harm"),
left to review. The numbers for weighing a different bound: the GPU tier's
steady state is 620 + bound + 30 (the read), so the largest bound that
keeps it under the 896 line is **245**, which would serve a pure-Rust
flush of up to **252** fd-carrying requests (28 x floor(bound / 28) + 28)
and put the drain-window figure at 1151. The bound is the fork's
`MAX_QUEUED_FDS`; changing it means a new fork commit and repin.

### What is left

- **The software-GLES drain window.** One connection at its 512 bound, with
  renderer copies of released planes awaiting a drain (up to 256 more) and
  128 parked, is 1004 at rest, past the line for that moment, and 1034
  inside the read past the bound, past the 1024 table. What that instant
  would do is reasoned in `fd_pressure.rs`, not measured.
- **Connection multiplication got cheaper.** Parked fds need no objects and
  the pressure grace never sees them, so each idle connection can hold 129.
  Measured (dev VM, headless pixman, 1024-fd table, `~/evidence/fdq/runs/many-connections-fork-8b01249.out`):
  6 connections parking 128 each held scoot at 792 fds and everything was
  served; 7 took it to 921, where newcomers got 0 globals and `scootctl`
  was refused under fd pressure; 8 filled the table (1024) and `scootctl`
  got a connection reset. None of them was disconnected. Before the fork,
  one connection could do the same. Recorded on
  [`pressure-many-light-connections`](../core/pressure-many-light-connections.md).
- **The parked-syncobj over-count** (`client_fds.rs`, "A known over-count")
  is bounded by the queue bound, not removed.
- **Outgoing fds** (an fd-carrying event queued for a client that stops
  reading) are unchanged in the fork and still reasoned, not measured
  (`fd_pressure.rs`).

### Evidence

Dev VM (kernel 6.18.50, `RLIMIT_NOFILE` 1024), debug builds, all under
`~/evidence/fdq/`. Before: `main` `173029d` (`bin/scoot-main-173029d-debug`,
sha256 `920e6c8d…`). After: branch commit `8b01249`
(`bin/scoot-fork-8b01249-debug`, sha256 `73a4f47c…`; only the fork build
contains the string "too many file descriptors queued").

- **The PR #236 reviewer's probe, unmodified** (`~/review-cfb/stuff-run.sh`,
  35 `wl_display.sync` requests carrying 28 `/dev/null` fds each):
  `main` 18 → **999** fds, newcomer `wayland-info` 0 globals, `scootctl`
  refused under fd pressure, the attacker connected to the end. Fork: the
  attacker is disconnected after its fifth message, scoot stays at **18**,
  `wayland-info` sees 38 globals, `scootctl version` answers
  (`runs/orig-probe-{main-173029d,fork-8b01249}.out`).
- **The same probe, extended to print the error it gets** (`fdstuff2.c`,
  40 messages): `main` fills the table (1024, `EMFILE` sheds for both
  sockets, `scootctl` connection reset); fork: `WL_DISPLAY_ERROR object=1
  code=1 message="too many file descriptors queued (more than 128)"`, EOF,
  18 fds (`runs/failfirst-main-173029d.out`, `runs/after-fork-8b01249.out`).
- **The research probe crate on Linux** (`fdcap-test/`, built against the
  fork at exactly `a39311b`, then against crates.io 0.3.17 as the control),
  with the new back-to-back variants (every chunk written before the server
  dispatches): 128 fds ahead of their bytes served; 129 with the final write
  carrying the bytes served; 129 whose bytes never arrive killed; a Rust
  flush of 140 served, 141 killed. The control keeps all of them connected
  (`runs/fdcap-fork-a39311b.txt`, `runs/fdcap-unpatched-0.3.17.txt`).
- **scoot's own tests, fail-first:** with the `[patch]` entry removed (tree
  otherwise as `c6050db`), three of the new tests fail: the attack test
  and the exact-bound test with the client still connected (140 and 129
  fds parked), and the 141-request batch test with the batch served
  (`runs/failfirst-tests-patch-removed.txt`); with it, all pass. The
  read-size and 140-batch tests pass on both, as they should: neither is
  the fork's behaviour.
- **Real clients** (`legit.sh`, headless, pixman and GLES, fork and `main`
  side by side): foot, zenity (GTK 4), es2gears, vkcube and mpv (`--vo=gpu`)
  all map and run with identical fd counts on both builds, and no protocol
  error is logged. mpv `--vo=dmabuf-wayland` fails to find an output format
  on both builds identically (headless has no NV12 path). scoot `--nested`
  inside a forked scoot (a pure-Rust client against the forked server), pixman
  and GLES: foot maps inside, both screenshots are correct, no protocol
  error on either side (`runs/nested-*-fork-8b01249.*`).
- **Smoke:** headless `rc=0` (23 `ok:`). Nested in cage: 20 `ok:`, then
  the xwayland step fails with "binary present never became ready",
  identically on `main`: cage puts `Xwayland` on the script's `PATH`, and
  the debug build under test has no `xwayland` feature (its log says so).

## The original report


Filed 2026-09-24 from the review of PR #236 (client-held fd bounds).
Pre-existing: it behaves identically on `618b5dc`, before that PR. Serves
**both** priorities, because a single misbehaving client can make scoot turn away
every new client, `scootctl` included, which cuts off the agent's own
control channel.

### What is wrong

wayland-backend 0.3.17, the Rust server implementation scoot uses through
Smithay, queues the fds that arrive with a client's messages in a
per-connection `in_fds: VecDeque<OwnedFd>` (`src/rs/socket.rs:135`). It
has no bound. Fds leave the queue only when a request whose signature
carries an fd argument is parsed. Fds that arrive alongside requests
without one stay queued for the life of the connection.

None of scoot's per-client caps can see this. Those caps count objects scoot
creates (buffers, pools, params planes, timelines); these fds never reach
scoot's code. So fd pressure's reserve fills up, new connections are shed at
accept, and the pressure kill never picks the holder, because its counted
creations are not over grace.

libwayland (the C implementation) disconnects a client that sends more fds
than it can account for. wayland-backend does not.

### Measured (PR #236 reviewer, dev VM, headless pixman, 1024-fd table)

- scoot went from 18 to 999 open fds, all held for one idle connection.
- New `wayland-info` connections got 0 globals. `scootctl` was refused
  under fd pressure.
- The holding client stayed connected and was never killed. scoot used no
  CPU while holding the fds and dropped back to 18 once the client exited.
- Identical before and after PR #236. The probe source and runner are on
  the dev VM in `~/review-cfb/`.

### Knock-on (reasoned, not demonstrated end to end)

The per-client fd ledger (`client_fds.rs`, which replaced
`drm_syncobj/retained.rs`) decides whether a retained timeline is still
held by checking whether the recorded fd number is still an open syncobj.
A syncobj fd parked in this queue could land on a number another client's
timeline used to occupy and be counted as that client's, inflating its
retained count and refusing it. Fixing the queue bound removes the
precondition. (Pool and plane records are immune: they are checked by the
file's identity, which a different file does not share. Every syncobj
shares one anonymous inode, so timelines cannot be.) How a fix here would
plug into that ledger's per-client total is in
[buffer-fds-past-their-object](../resolved/buffer-fds-past-their-object-done.md#the-interface-for-the-wayland-backend-fix).

### Fix routes (as filed; the user chose 2)

1. **Upstream:** a bound (or libwayland-style disconnect) on the received-fd
   queue in wayland-rs. Nothing is filed upstream from this project (`docs/forks.md`); don't
   draft upstream text (see `CLAUDE.md` and the Smithay AI-policy note;
   check wayland-rs's own contribution policy first).
2. **A scoot-carried fork** of wayland-backend with that bound, pinned the
   same way as the Smithay fork (`scoot-sh/smithay`, see
   [smithay-fork-repin](./smithay-fork-repin.md)).
3. **Scoot-side mitigation**, if one exists: e.g. per-client fd attribution
   from `/proc/self/fd` would let fd pressure find the holder even though
   the fds aren't counted. Evaluate before choosing 1/2.

### Evidence expected (as filed)

Fail-first: the reviewer's probe fills the table on current `main`. After
the fix, the holding client is disconnected (or bounded), and newcomers and
`scootctl` keep being served. Legitimate clients are unaffected (real
clients never send fds on fd-less requests). Standard gate.
