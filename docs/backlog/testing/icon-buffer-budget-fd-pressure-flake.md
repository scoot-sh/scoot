---
title: "Flake: the icon live-buffer-budget flood is refused by process-wide fd pressure under `cargo test` — LOW, load-only, pre-existing."
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Flake: the icon live-buffer-budget flood is refused by process-wide fd pressure under `cargo test`

Found alongside the [spawn fd-identity
flake](../resolved/spawn-fd-number-identity-flake-done.md) (2026-09-19), not
by that change's diff — reproduced at pristine `main` (`2162da8`), with that
PR's tree never failing it more often. Filed rather than fixed: it is a
different test, a different mechanism, and outside that ticket's scope.

## What happens

`compositor::toplevel_icon::tests::icon_buffers_fill_the_same_live_buffer_budget`
times out at `toplevel_icon/tests.rs:410` under `cargo test` (never under
`cargo nextest run`, which gives each test its own process):

```
Protocol error 1 on object wl_shm_pool@387: wl_buffer refused: compositor-wide
file-descriptor pressure, and this client holds more than the 128-buffer
pressure grace

thread '...::icon_buffers_fill_the_same_live_buffer_budget' panicked at
crates/scoot/src/compositor/toplevel_icon/tests.rs:410:13:
timed out waiting for a client step acknowledgement; the client thread stopped
or the compositor did
```

The client is killed by the refusal, so it never acknowledges the step and
the fixture waits out its 10s `PATIENCE` — the +10s is visible in the run
time (21s green, 31-35s red).

## The mechanism

The test deliberately floods 512 icon buffers to fill
`MAX_BUFFERS_PER_CLIENT`, and takes `hold_flood_lock()` so two *flood* tests
cannot run at once. But the fd-pressure ceiling it runs into is not per-test:
`fd_pressure::table()` reads the **process's** `/proc/self/fd` count against
`RLIMIT_NOFILE`, and under `cargo test` every other test in the binary is
holding fds in that same process. The flood sits close to the boundary by
design, so a few fds held by a neighbour at the wrong moment are enough to
make the refusal land inside the flood instead of after it.

The dev VM's `ulimit -n` is 1024, which is what makes the boundary reachable
there at all; the failure has not been seen in CI.

## Evidence (dev VM, 2026-09-19)

Two debug test binaries built from the two trees, run directly (so the
composition is identical run to run), `cargo test`'s own default
parallelism:

| tree | full-binary runs | icon-test failures |
|---|---|---|
| `main` @ `2162da8` | 8 | 2 |
| the spawn fd-identity branch | 12 | 6 |

The rate tracks the machine's background load, not the tree: every branch
failure fell in one window during which another agent was building and
running `--tty`/GLES sessions on the same 4-vCPU VM, and in that same window
a control run of the branch binary with the spawn test *skipped* failed 3
times in 4 — so the spawn test is not what tips it. In the batch run
strictly alternating the two binaries under identical conditions, `main`
failed 1/5 and the branch 0/5. The branch's own spawn test never failed in
any run.

## Disposition

Not fixed under the spawn ticket. Candidate directions, cheapest first:

- Have the flood test read the live fd headroom (`fd_pressure::table()`) up
  front and refuse to run — or raise `RLIMIT_NOFILE` for itself — rather than
  assuming the whole budget is reachable while neighbours hold fds.
- Or assert the *boundary* relatively (refused after N, whatever N the
  headroom allows) instead of at the absolute 512.
- The structural fix `CLAUDE.md` already names for this class — tests that do
  not depend on process-shared state — applies here too: this one depends on
  a process-global resource, which no lock between flood tests can fence off.
