---
title: "Flake: parked-captures poll sees an extra frame_serial advance under full-suite load — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: parked-captures poll sees an extra frame_serial advance under full-suite load — DONE (PR #110)

## The entry as filed

> Found by the PR #107 review (2026-09-18), not by that PR's diff: the
> reviewer's 9 back-to-back full `nextest --workspace` runs on the dev VM
> failed twice (~2/9) in
> `screencopy::tests::parked_captures_on_two_sessions_are_all_delivered_by_one_confirm`
> (`screencopy/tests.rs:777`): after `MapWindow`, a re-parked capture polls
> `Ready` where the test asserts `Waiting` — `frame_serial` advanced one
> extra time between park and poll.
>
> Why it is not PR #107: the failing file/assertion is untouched by that
> diff, and the diff is provably behavior-neutral in the harness (no `Tty`
> exists without DRM, so `retry_render` stays false; the only headless
> change is a never-taken branch). The test passes 15/15 in isolation on
> the same HEAD; it fails only under full-suite parallel load — a
> commit-coalescing/timing shape (a second damage event landing in a later
> dispatch under contention), not a logic shape. The reviewer could not run
> it on pre-PR `main` (read-only role), so pre-existence is established by
> independence, not by bisection.
>
> Disposition: the poll's "no extra serial advance between park and poll"
> assumption needs pinning or a settle before the poll. Merged with
> disclosure in PR #107 rather than blocking it or re-running till green.

## Resolution

Test-only fix in `screencopy/tests.rs` (no production code touched —
`git diff` is one test file, +59/−3). The delivered-frames assertions after
the confirm stay byte-exact; only the intermediate `Waiting` poll learns to
re-sync. No README change (no user-facing surface).

**The filed mechanism is corrected, not just fixed.** Verify-first
reproduction plus per-step `frame_serial` instrumentation showed the
failing shape is `Ready` at an *unmoving* serial — park, settle and poll
all read the same value (measured `3, 3, 3` across six caught trips), so no
extra advance happens between park and poll. What lags is the session's own
`delivered` marker: under parallel load the pre-map parked frame can be
consumed by an earlier tick than the map's own, so at re-park time
`delivered != Some(serial)` and the re-parked capture is *due* — serving
it is correct production behavior (`Capture::due`, `screencopy.rs:460`),
not a race. The test's "re-parked captures are not due" assumption is what
was false. Disposition unchanged (fix the test, not production), mechanism
updated: `delivered`-lag at park time, not a serial advance after it.

Two halves, both test-only (the first alone proved insufficient: a
quiescence-only cut still tripped 1/30 under load, which is what sent the
investigation to the per-step serial instrumentation that found the real
`delivered`-lag mechanism):

- **Quiescence before re-park.** A stable `frame_serial` across a settle
  plus a 50ms tick (bounded at ten rounds, asserted afterwards) means no
  commit is still working through the frame timer: both clients are parked
  on their step channels and no timer is armed at idle, so nothing after
  it can advance the serial again.
- **Synchronize-then-assert retry.** Each re-park polls up to three times:
  a `Ready` drains the lag (re-syncing `delivered` to the current serial)
  and re-parks, which then waits deterministically. One retry is the
  proven max; the bound fails loudly instead of polling forever, and a
  retry prints a one-line `eprintln` naming client, attempt, outcome and
  serial. A `Failed` outcome is never retried past the assert — it would
  indicate a real bug.

### Evidence (dev VM, `ssh -p 2222 dev@localhost`, `CARGO_TARGET_DIR=/var/cargo-target`)

- **Reproduced pre-fix:** 5/5 green in isolation; 6/30 trip under load
  (targeted loop concurrent with 3× full-suite `nextest` runs), every trip
  at `tests.rs:777` with `left: Ready, right: Waiting`.
- **Production pin still discriminates:** with `confirm_lock`'s
  `ensure_ticking()` temporarily removed, the fixed test fails at the
  `timer_armed` pin (not at the re-park poll); reverted after.
- **Retry path proven live:** a temporary probe mapping a second window
  after quiescence forced lag on both sessions — both attempt-0 polls came
  back `Ready`, both retries parked `Waiting`, full test green; probe
  reverted (not in the diff).
- **Post-fix stress, final tree:** 40/40 targeted iterations plus 12/12
  full test-binary runs under the same 3-process oversubscription that
  tripped 6/40 + 1/12 pre-fix (zero retries fired — the lag shape did not
  recur in the sample); `cargo test -p flexwm` 907+3, `cargo nextest run
  --workspace` 1012 passed (×3 consecutive on the final tree), `cargo
  clippy -p flexwm --all-targets -- -D warnings` clean, `cargo fmt --check
  -p flexwm` clean, `scripts/smoke-test.sh` 17 ok rc=0.

### Found alongside (filed separately, not fixed here)

One full-binary run under the same abusive oversubscription failed in an
unrelated suite:
`activation::tests::keyboard::an_activation_takes_the_keyboard_back_from_a_clicked_taskbar`
(`keyboard.rs:304`, "the click never reached the taskbar"). 5/5 in
isolation on the same tree; never observed under the standard suite. Same
family (a single `settle()` insufficient under load — the file already
names the shape), different test. Filed as
[`activation-taskbar-click-settle-flake`](../protocols/activation-taskbar-click-settle-flake.md)
(low, load-only).
