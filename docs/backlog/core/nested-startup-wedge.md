---
title: "Nested scoot waits forever at startup against a host that accepts but never answers"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Nested scoot waits forever at startup against a host that accepts but never answers

Filed 2026-10-06 from the PR #467 review. Serves **computer use**: the
Selkies webtop container runs `scoot --nested` under s6, and an agent
there loses IPC for good when this happens.

## The gap

PR #467 made nested scoot stop loudly (exit 1) when its host connection
breaks, so s6 restarts it. The review reproduced the follow-on in the
real `image-webtop-scoot` on the M2 (`kill -9` of the Selkies parent):
s6 restarted both sides, but the restarted host listened without ever
dispatching (its `wayland-1` Recv-Q grew 14 → 26).

The restarted scoot then blocked in `nested::init`'s registry roundtrip,
which has no timeout. It stayed alive for more than 100 s with no
`scoot is up` and no IPC socket, so `scoot msg` was refused for good,
and the supervisor never saw an exit to act on.

## What to do

- Bound the startup roundtrip(s) to the host: for example a few seconds,
  then a loud error naming the host and exit 1, the same as #467's
  post-startup loss, so a supervisor retries. Pick the limit with a
  reason: a slow but healthy host (a loaded VM) must still start.
- A test with a host socket that accepts and never answers. It must fail
  (hang, bounded by the test's own timeout) before the fix.
- While there, two small leftovers from #467's review:
  - `crates/scoot/src/compositor/nested.rs:102` and
    `nested/tests.rs:174` still cite
    `docs/backlog/ipc/nested-ipc-socket-refuses.md`; it is now
    `docs/backlog/resolved/nested-ipc-socket-refuses-done.md`.
  - The host loss logs twice per death: once from `before_sleep`'s flush
    and once from `process_events`. Log only on the first mark.

## Not in this ticket

What kills the Selkies parent in the first place (Selkies-side; the #30
benchmark containers that showed it are gone).
