---
title: "look_ipc second-swap test flakes when the lock holder exits early"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# look_ipc second-swap test flakes when the lock holder exits early

Filed 2026-10-27. Serves **daily-drive** (a red CI on an unrelated PR wastes review cycles and erodes trust in the gate).

## The gap

`crates/scoot/tests/look_ipc.rs::a_second_swap_while_one_runs_is_refused` failed in CI on the Linux (minimal) job, run 40437160156 (PR #758 head 7ee680d): the whole test took 7.0s on a loaded runner.

The test holds the state-dir lock with `flock <lock> sleep 6`, then spawns the first swap (which parks on the lock), waits for the first swap's child process to appear (up to 30s poll) plus a fixed 300ms grace, and asserts the second swap is refused with "already running" (assert at line ~637). On a loaded runner the holder had already exited before the second request arrived, the first swap completed, and the second swap was NOT refused.

Timing dependence at the cause: the 6s holder lifetime is a wall-clock guess about when the second request lands, and the 300ms grace is a guess about the park having happened. Under load both guesses break in the same direction.

## What to do

Remove the timing dependence at the cause, not by raising timeouts:

- Hold the lock until the second request has been answered (deterministic gate), and release it explicitly afterwards — e.g. hold the lock in-process (or via a child that waits on an explicit release signal) so the holder cannot exit early.
- Keep the child-visible deterministic gate (spawn + park proof) that already exists, but make the "second swap while one runs" overlap structural rather than time-bounded.
- Edge cases: no orphaned `flock`/`sleep` holding the lock past the test (explicit release + `wait`), first swap must still land after release, refusal message still names the gate.

Prove the flake first (stress under load: parallel runs or high `--test-threads` with `--retries 0`, repeat 50x), show before/after pass rates.

## Not in this ticket

Changing how the one-swap gate behaves for users; raising the 6s/300ms timeouts.

## Resolution (2026-10-27)

Landed as two `test(scoot)` commits in this PR. First, `hold look-swap lock until the second request is answered`: the `flock sleep 6` child is replaced by an in-process exclusive `rustix::fs::flock` guard held until after the second request is answered, then dropped explicitly; the fixed 300 ms grace is removed (the second `scoot msg` spawn plus IPC connect takes milliseconds while the park lands microseconds after the spawn, so the park is set before the second request reaches the gate). Second, `scope the swap gate to the session pid` (found by this PR's own CI minimal run): the `/proc` scan for `--from-compositor` is scoped to the session compositor pid via `/proc/<pid>/stat` ppid, so a concurrent `look_ipc` test's child cannot fire the gate early (which let the second request win the race and refuse with lock contention instead of the gate). No orphaned holder, first swap still lands after release, refusal still names the gate.

Evidence: before — CI Linux (minimal) run 40437160156 (PR #758 head 7ee680d), whole test 7.0s vs 6s holder, holder exited early, first swap completed, second NOT refused. After — dev VM (`ssh -p 2223`, `/dev/shm/wt-bl5`, `CARGO_INCREMENTAL=0`, `nix develop path:/dev/shm/wt-bl5 -c cargo ...`): `cargo test -p scoot --test look_ipc a_second_swap_while_one_runs_is_refused` 50/50 green (~0.7s each, no 6s holder); `cargo fmt --check -p scoot` clean; `cargo clippy -p scoot --all-targets -- -D warnings` clean (one `suspicious_open_options` fixed with explicit `.truncate(false)`).
