---
title: "Nested scoot waits forever at startup against a host that accepts but never answers"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolved 2026-10-06 (PR #469)

Every blocking host wait before the event loop was checked: only two
exist, both now bounded. `registry_queue_init` (every build) always
blocked; `gpu::read_feedback`'s `queue.roundtrip` blocks only in
`gpu-scanout` builds with a GLES renderer. `BufferPool::new` (shm) only
mints objects without waiting, and the first-configure wait is already
async (no roundtrip before the loop).

What landed in `fix(scoot): bound nested startup wait on its host`:

- `nested::STARTUP_TIMEOUT` (10 s) bounds each startup roundtrip on a
  helper thread (the roundtrip blocks uninterruptibly in `poll`, and
  reimplementing the registry fetch would duplicate `wayland_client`
  internals). Expiry logs `could not reach the host compositor during
  startup` naming the cause and returns `Err`, which the process reports
  as `scoot: ...` with exit 1 -- the same contract as #467's loss, so a
  supervisor retries. No env/CLI override: one knob-free bound.
- The dma-buf feedback timeout falls back to read-back (never fatal):
  nothing past it blocks, and `HostSource` still names a truly dead
  host loudly on its first dispatch.
- The two leftovers: both stale comment paths repointed at the resolved
  entry, and `HostLoss::mark` now answers first-or-not so the loss logs
  exactly once per death (measured 2 → 1 in a live host-kill).

Why ten seconds: healthy nested startup against a headless host on the
Asahi M2 measures ~60 ms idle and the same at load average 5.1 (while
`cargo build --workspace` runs), so 10 s is ~170x with room for a slow
but answering host; it matches the suites' existing `HOST_PATIENCE`.

Evidence (all on the M2, own dirs/target/runtime; soft-egl with correct
quoting): reproduced the wedge pre-fix with an accept-and-never-answer
host (nested alive 15 s, no `scoot is up`, rc=124 under `timeout`);
post-fix the same host gives rc=1 in 10 s with the loud line, and a live
host-kill gives rc=1 with one loss log and no calloop jargon. New test
`starting_against_a_silent_host_fails_loud_within_the_bound` fails
pre-fix (30 s parent deadline, never hanging nextest) and passes
post-fix (~10 s), 30/30 flake-clean; full `nextest --workspace` shows
only the known environmental scootbar set (22/22 baseline match plus
one consequential snapshot-meta check, zero elsewhere);
`cargo test -p scoot`, clippy, fmt, smoke and the site build are green.
One detour is recorded here deliberately: a first fork-based version of
the test broke unrelated `layer_shell` teardown tests in the
shared-process `cargo test` run (4/4 red with it, green without and on
baseline), so the test re-executes this binary as a subprocess instead
-- a fresh process shares nothing with concurrent fixtures.
