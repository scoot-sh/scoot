---
title: "Flake: the icon live-buffer-budget flood is refused by process-wide fd pressure under `cargo test` — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: the icon live-buffer-budget flood is refused by process-wide fd pressure under `cargo test` — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). The flood runs
under a raised `RLIMIT_NOFILE` ceiling with verified headroom, the 513rd
refusal is pinned to the budget by its message, and the counting invariant
is pinned a second time by a deterministic test that opens no fd at all.

## The entry as filed

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

## Resolution: raise the ceiling, split the invariant, pin the message

Test-only (`compositor/toplevel_icon/tests.rs`); no production code changed.
The fix takes the ticket's first and third directions together and declines
the second, for the record:

- **Raise, then verify (first direction).** The flood test calls
  `ensure_flood_headroom()` under the existing `hold_flood_lock()`: it
  raises the process's soft `RLIMIT_NOFILE` to 4096 (capped at the hard
  limit, never lowered), then re-reads `fd_pressure::table()` and requires
  896 free (512 flood + 128 reserve + 256 neighbour slack). Raising is the
  safe direction for every neighbour sharing the `cargo test` process —
  `free = soft - used` only grows, so parallel tests see fewer pressure
  verdicts, never more — and it cannot blind the suite's own pressure pins,
  which drive hand-built tables or a forked child's own copy of the limit,
  never the live process table.
- **Split the invariant (third direction).** New
  `the_live_buffer_budget_counts_to_512_with_no_flood_at_all` drives
  `WlBuffers::claim_buffer_creation` straight through a server-side client
  that never binds anything: 512 claims admitted, the 513rd refused, the
  count drained to zero via `forget_buffer`, no fd opened anywhere. The
  count no longer depends on process-shared state at all; the flood keeps
  the integration shape (real icon-factory wire buffers sharing the one
  budget).
- **The relative boundary (second direction) is declined.** Asserting
  "refused after N, whatever N the headroom allows" would stop pinning the
  absolute 512 the DoS bound is written in — a green suite that no longer
  guards the bound, which the ticket already names as worse than the flake.
- **The 513rd refusal is now pinned to the budget by message.** Budget and
  pressure refusals post the same `wl_shm::Error::InvalidStride` on the
  same `wl_shm_pool` object, so the code cannot discriminate them; the
  test asserts the client-visible error contains
  `maximum of 512 live buffers` (the `too_many_buffers()` text), which
  only the budget refusal carries.

No skip path: where the table cannot deterministically fit the flood
(hard limit too low, `setrlimit` refused, headroom still short),
`ensure_flood_headroom` panics fast naming used/soft/need and the remedy
— never the 10s `PATIENCE` timeout, and never a passing skip, which would
stop guarding the bound. The deterministic counter test still pins the
invariant on such a machine.

## Evidence (dev VM, 2026-09-20)

Pre-fix reproduction at `main` (`1ef3362`), the ticket's own method (debug
test binary run directly, `ulimit -Sn` 1024 / `-Hn` 524288, 4 vCPU).
Binary `/var/cargo-target/debug/deps/scoot-9de053b31125ddf7`:

| runs | mode | icon-test failures |
|---|---|---|
| 8 | full binary, default parallelism | 0 (flake did not show; ticket's baseline at `2162da8` was 2/8 — load-dependent) |
| 10 | full binary, `--test-threads=16` | 0 on the icon test (1 run failed 2 unrelated `dispatch` flood tests with `BrokenPipe` — same shared-table class, different tests, out of scope) |
| 1 | induced: `prlimit --nofile=700:700`, icon test only | **1 — the exact ticket signature**: `wl_buffer refused: compositor-wide file-descriptor pressure, and this client holds more than the 128-buffer pressure grace`, panic at `toplevel_icon/tests.rs:410`, 10.21s (`/tmp/prefix-induced-700.log`) |

Post-fix, same binary path rebuilt from the branch (final tree; clippy
`-D warnings` and `fmt --check` clean):

| runs | mode | result |
|---|---|---|
| 8 | full binary, default parallelism | 8/8 green, 1080 passed each, ~21s each |
| 6 | full binary, `--test-threads=16` | 6/6 green, 1080 passed each, ~7s each |
| 1 | induced: `prlimit --nofile=700:700`, icon test only | fast loud panic (0.00s, not the 10s timeout): `no deterministic headroom for the icon-buffer flood: table is 4/700 used/soft, need 896 free (512 flood + 128 reserve + 256 neighbour slack); raise this process's RLIMIT_NOFILE hard limit` |
| 1 | `cargo nextest run --workspace` (final tree) | 1211 passed, 4 skipped |

The natural flake did not reproduce pre-fix in 18 full-binary runs
(load-dependent, and the VM was quiet), so the post-fix rate is stated
against the ticket's 2/8 baseline plus the induced proof: the mechanism
is byte-identical pre-fix under a tightened table, and post-fix the same
tightened table fails in 0.00s with the headroom numbers instead of
burning the 10s `PATIENCE`.

Out of scope, noted not fixed: the two `dispatch` flood tests that failed
once under `--test-threads=16` pre-fix (`flooding_single_pixel_buffers_…`,
`a_dmabuf_create_past_a_full_budget_…`), and the spawn fd-identity area
the ticket already excludes. No VM hardware beyond test runs was needed.
