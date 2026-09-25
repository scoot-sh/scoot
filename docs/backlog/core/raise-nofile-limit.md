---
title: "Raise scoot's RLIMIT_NOFILE soft limit to the hard limit at startup (children get the original back), then give the wayland-backend fd-queue cap libwayland parity"
status: "open"
area: "core"
priority: "high"
blocked: "after PR #241 (wayland-backend fork repin) merges"
---

# Raise the fd limit; loosen the queue cap to libwayland parity

Filed 2026-09-24 (coordinator), from PR #241. Serves **both** priorities:
it removes the root cause behind every tight margin in the fd-pressure work,
and it lets the fork's queue cap stop disconnecting clients libwayland would
serve.

## Why

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

## What to do

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

## Evidence

`/proc/<scoot>/limits` raised and `/proc/<child>/limits` original; the
PR #241 legit-client batching probe (141/200 per flush) now served; the
many-idle-connections run no longer sheds `scootctl`; fd-pressure tests
at the new table size; the containers case (hard=1024) unchanged.
