---
title: "wayland-backend keeps a client's received fds in an unbounded queue: one connection can fill scoot's fd table"
status: "open"
area: "core"
priority: "high"
blocked: "needs a user decision on the fix route (upstream wayland-rs change vs a scoot-carried fork)"
---

# Unbounded per-connection received-fd queue (wayland-backend)

Filed 2026-09-24 from the review of PR #236 (client-held fd bounds).
Pre-existing: it behaves identically on `618b5dc`, before that PR. Serves
**both** priorities, because a single misbehaving client can make scoot turn away
every new client, `scootctl` included, which cuts off the agent's own
control channel.

## What is wrong

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

## Measured (PR #236 reviewer, dev VM, headless pixman, 1024-fd table)

- scoot went from 18 to 999 open fds, all held for one idle connection.
- New `wayland-info` connections got 0 globals. `scootctl` was refused
  under fd pressure.
- The holding client stayed connected and was never killed. scoot used no
  CPU while holding the fds and dropped back to 18 once the client exited.
- Identical before and after PR #236. The probe source and runner are on
  the dev VM in `~/review-cfb/`.

## Knock-on (reasoned, not demonstrated end to end)

`drm_syncobj/retained.rs` decides whether a retained timeline is still
held by checking whether the recorded fd number is still an open syncobj.
A syncobj fd parked in this queue could land on a number another client's
timeline used to occupy and be counted as that client's, inflating its
retained count and refusing it. Fixing the queue bound removes the
precondition.

## Fix routes (user decision needed)

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

## Evidence expected

Fail-first: the reviewer's probe fills the table on current `main`. After
the fix, the holding client is disconnected (or bounded), and newcomers and
`scootctl` keep being served. Legitimate clients are unaffected (real
clients never send fds on fd-less requests). Standard gate.
