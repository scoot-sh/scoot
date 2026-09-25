---
title: "wayland-backend kept a client's received fds in an unbounded queue: one connection could fill scoot's fd table — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Unbounded per-connection received-fd queue (wayland-backend) — RESOLVED

RESOLVED 2026-09-24 (PR #PRNUM) by route 2 below: scoot now builds against
a scoot-sh fork of wayland-backend (`docs/forks.md`), pinned through the
root `Cargo.toml`'s `[patch.crates-io]` at `a39311b8` (the 0.3.17 release
`72f7fe0d` plus one server-side commit). Nothing was filed upstream.

## What changed

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
  by `cmsghdr` alignment), and a Rust `wayland-client` flush of 140
  fd-carrying requests is served while 141 is disconnected.
- **fd pressure's arithmetic** (`fd_pressure.rs`) adds the queue: 128 at
  rest, 158 for a moment inside the read past the bound.

## What it costs a legitimate client (a finding, not hidden)

The check counts fds a client has sent ahead of their requests. A client
on the Rust `wayland-client` (rs backend) grows its outgoing buffer without
limit and, on flush, sends every fd past the last 28 ahead of the bytes, 28
per `sendmsg` with one byte each. So a Rust client that queues **more than
140 fd-carrying requests between flushes** (pools, planes, timelines, gamma
ramps, selection receives) now has 140 queued before the final write and is
disconnected. 0.3.17 served it. Measured on the dev VM with a standalone
Rust client against the real binary: 28, 100, 128 and 140 served, 141 and
200 disconnected with the fork's error (`main` served all six). One scoot
test hit it: `gamma_rapid_sets_stay_alive` queued 200 `set_gamma` before
one round trip; it now flushes every 64 ramps (still back to back, which is
what it tests). No libwayland client can hit it, and no real client seen
batches that many.

The bound cannot simply be raised to cover larger batches: with 158 on top
of one connection's 620, the GPU tier sits at 778 against the 896 pressure
line, and a bound of 256 would put it past.

## What is left

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

## Evidence

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
  inside a forked scoot (the Rust client against the forked server), pixman
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
