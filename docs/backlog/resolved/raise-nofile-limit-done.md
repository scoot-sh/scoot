---
title: "Raise scoot's RLIMIT_NOFILE soft limit at startup (children get the original back); fd-queue cap to libwayland parity — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Raise the fd limit; queue cap to libwayland parity — RESOLVED

RESOLVED 2026-09-25, folded into PR #241 by the coordinator's decision
after review of that PR found stock libwayland clients reach the fixed
128-fd queue cap under backpressure (see
[the queue record](./wayland-backend-fd-queue-done.md)).

## What changed

- **`crates/scoot/src/compositor/nofile.rs`.** `raise()` lifts the soft
  `RLIMIT_NOFILE` to the hard limit capped at **65536**, once, at the top of
  `compositor::run` (and, a no-op there, from `State::new`, so test
  harnesses run with a session's limit), and logs before and after with
  the resulting unclaimed-fd cap. It never lowers a soft limit.
- **Why 65536.** Raising costs nothing until fds are used (the kernel grows
  the fd table on demand) and epoll costs per registered fd. The cap bounds
  what local clients can make scoot hold before fd pressure sheds (the line
  is the table minus 128), and the cost of observing the table: a readdir
  of `/proc/self/fd` measured at 72 us for 1000 open fds, 634 us for 8000
  and ~8 ms for 65000 (`~/evidence/fdq/runs/readdir-cost.txt`). 65536 is 64
  connections at every per-client bound including the 1024 queue cap.
- **Children get the original soft limit** (hard limit unchanged):
  `restore_for_child` adds a `pre_exec` `setrlimit` to `State::spawn`'s
  `Command`, the one path keybindings, IPC `spawn`, `[autostart]` and the
  session command take.
- **XWayland is the exception, measured.** Smithay builds the server's
  `Command` with no hook, and the first version of this change put the
  original limit back around `XWayland::spawn` instead. Measured on the dev
  VM, the server still ran at soft 524288, its hard limit: Xwayland raises
  its own limit at startup (`try_raising_nofile_limit` in upstream
  `hw/xwayland/xwayland.c`, unless `-lf` is given). The toggle was removed
  (`7ce06df`): it lowered scoot's whole limit for the spawn, for nothing.
- **A hard limit of 1024** leaves everything as it was, with a startup log
  line naming the 1024-table margins and the 128 queue cap.
- **fd pressure** already read the limit (`getrlimit`) for every line; its
  documentation and arithmetic test now cover both tables.
- **The fork's queue cap** follows the limit: one eighth of the soft limit
  per client, clamped 128..=1024 (`scoot-sh/wayland-rs` `70f81e00`).

## Evidence

Under `~/evidence/fdq/runs/` on the dev VM, binary
`bin/scoot-raise-2c18a93-debug`:

- `/proc/<scoot>/limits` 65536 / 524288; `foot` (session command and IPC
  `spawn`) and its shell 1024 / 524288 (`limits-raise-2c18a93.out`).
- Container (`prlimit --nofile=1024:1024`): not raised, log line, children
  1024 / 1024, cap 128 (`limits-container-raise-2c18a93.out`,
  `container-attack-raise-2c18a93.out`).
- The first round's batching probe shapes are now served: a pure-Rust
  client's one flush up to 1036 and the libwayland backpressure probe at
  160, 600 and 1000 (`bprun-160-600.txt`; the in-process pins in
  `fd_pressure/tests/backend_queue_client.rs`).
- Many idle connections: 8 no longer shed `scootctl` (8218 fds, served); it
  takes 64 parking 1024 each to fill the table
  (`many-connections-raise-2c18a93.out`).
- `nofile/tests.rs`: a child through `State::spawn` reads the original soft
  limit and the unchanged hard limit; it fails without the restore
  (`failfirst-child-limit-restore-removed.txt`: the child read 65536).

## The ticket as filed

Filed 2026-09-24 (coordinator), from PR #241. Serves **both** priorities:
it removes the root cause behind every tight margin in the fd-pressure work,
and it lets the fork's queue cap stop disconnecting clients libwayland would
serve.

### Why

scoot runs with the default soft `RLIMIT_NOFILE` of 1024 (the dev VM's hard
limit is 524288). Every fd-exhaustion fix in PRs #236/#239/#241 is sized
against that 1024-entry table (the 896 pressure line; the 577/620/748
single-client figures; 7–8 idle connections parking 128 fds each fill it).
PR #241's queue cap (128 unclaimed fds per connection) therefore has to be
small, and so it disconnects a legitimate pure-Rust-backend client that
queues more than ~140 fd-carrying requests in one flush, which `main`
served and libwayland (whose bound is 1024) would serve.

Compositors raise the soft limit to the hard limit at startup and restore
the original soft limit in children they spawn (niri does this; programs
using `select()` break with fds ≥ 1024, so children must get the original
back). scoot spawns clients (`spawn`, `[autostart]`, `session.command`,
XWayland), so the restore must cover every spawn path.

### What to do

1. At startup, before any fd-heavy setup, raise the soft limit to the hard
   limit (cap at something sane if the hard limit is huge, e.g. 64k or
   1M: check fd table memory and `poll`/`epoll` costs). Record the
   original soft limit.
2. Restore the original soft limit for every child: the spawn paths in
   `State::spawn` / autostart / `session.command` / XWayland server
   launch (`pre_exec`), verified by reading `/proc/<child>/limits`.
3. Make `fd_pressure.rs` derive its lines from the *actual* limit, not a
   hard-coded 1024 (check whether it already does), and restate the
   arithmetic and docs.
4. Raise the fork's `MAX_QUEUED_FDS` to libwayland parity (1024, its
   default `fds_in` bound) with a new commit on `scoot-sh/wayland-rs`
   (update `docs/forks.md`), so no client libwayland would serve is
   disconnected. Keep the per-client ledger (#239) as the real bound.
5. Handle a hard limit that's already 1024 (containers): everything keeps
   working at today's margins, with a startup log line saying so.

### Evidence (as filed)

`/proc/<scoot>/limits` raised and `/proc/<child>/limits` original; the
PR #241 legit-client batching probe (141/200 per flush) now served; the
many-idle-connections run no longer sheds `scootctl`; fd-pressure tests
at the new table size; the containers case (hard=1024) unchanged.
