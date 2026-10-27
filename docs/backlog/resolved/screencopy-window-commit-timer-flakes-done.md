---
title: "screencopy/window_commit timer tests flake under parallel load"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# screencopy/window_commit timer tests flake under parallel load

Filed 2026-10-17, from the PR #655 fix-first review round. Serves **daily-drive**: red CI on unrelated PRs wastes review cycles and erodes trust in the gate.

## The gap

Two different things looked like one flake family. The `timer_armed` reds on the #655 head are explained and fixed: CI on that head failed twice in the screencopy timer asserts (xwayland-job nextest, compositor-job `cargo test`) at an elevated rate over main, and the dev-VM A/B pins the cause to the bind-time `connect` probe — probe-everything behavior went 6/6 RED (7 failures, all `timer_armed`-family: `screencopy::tests::a_confirm_with_no_parked_capture_costs_one_tick`, `a_parked_capture_...vblank...`, `...fallback...`) while the lock-check-first fix went 6/6 GREEN 58/58 shared-dir plus 3/3 private-dir. That half is done with the #655 fix round, not this ticket.

What remains is the base rate underneath: `screencopy parked_capture` (`.../outlives_locker`) and `window_commit desync_batch` fail intermittently under the single-process `cargo test` runner on both the fix head (the implementer's trio: these two plus `refused_cost`) and, in full-suite runs only, elsewhere. Targeted runs go green: fix head `cargo test` re-run 2733 passed / 1 failed (`refused_cost` only), main-baseline trio filter 9 passed / 1 failed (`refused_cost` only), main screencopy module 58/58. `refused_cost` itself is deterministic (256 buffers and 273 fds on main and head alike, same file:line) and already has its own ticket (`core/xwayland-refused-cost-fd-leak`).

## What to do

Characterize the base rate on `main` under full-suite load: run full `cargo test -p scoot --bin scoot` on the dev VM a few times, count `parked_capture` / `desync_batch` failures. If the base rate is confirmed, either fix the shared-process interference (both pass under nextest isolation, so this smells like cross-test state, not a product bug) or adopt the documented re-run-once policy for exactly these tests.

## Not in this ticket

The `refused_cost` fd leak (own ticket, since resolved as a swept-delta measure in #757); the probe-caused `timer_armed` reds (fixed in the #655 fix round).

## Resolution (2026-10-27)

Landed as `test(scoot): quiesce before parking screencopy captures` in this PR: `park_across_deferred_lock` and `a_parked_capture_outlives_a_locker_that_dies_mid_wait` now wait for a quiescent `frame_serial` (settle plus ticks until the serial holds, up to 10x, asserting stillness) before `CaptureWithoutWaiting`. This replaces the immediate park-after-Ready (a wall-clock guess that the screen is static) with an event wait, mirroring the proven pattern in `parked_captures_on_two_sessions_are_all_delivered_by_one_confirm` which already drains the same load shape with a retry. No timeouts raised (`POLL_PATIENCE` 300 ms and `tick(50 ms)` bounds unchanged); `desync_batch` has no wall-clock by design and is left untouched.

Evidence: before — ticket's trio: targeted runs green (2733 passed / 1 `refused_cost` only; main trio 9/1; screencopy module 58/58), full-suite intermittent `parked_capture`/`desync_batch`. After — dev VM (`ssh -p 2223`, `/dev/shm/wt-bl5`, `CARGO_INCREMENTAL=0`, `nix develop path:/dev/shm/wt-bl5 -c cargo ...`): `screencopy::tests::a_parked_capture*` (3 tests) 50/50 green; `window_commit::tests::batch` (3 tests) 20/20 green; full `cargo test -p scoot --bin scoot` 2831 passed / 0 failed / 42 ignored; `cargo fmt --check -p scoot` and `cargo clippy -p scoot --all-targets -- -D warnings` clean.
