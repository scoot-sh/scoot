---
title: "M1 review follow-ups: a clock that retries a failed draw, and four hardening gaps"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M2"
---

# M1 review follow-ups

Filed 2026-09-29 from the independent review of #324 (`module-api-and-clock`).
None of these blocked that merge; each is small and real. Serves **daily-drive**
(a bar that must not freeze) and the [robustness standard](robustness-and-limits.md).

## 1. A clock frozen after one failed draw (the one that matters)

`outputs.rs` `failed`: after one failed draw at a frame, later clock ticks never
retry until a configure changes the frame. Before the clock existed that only left
a stale solid color; now the clock stays frozen on that output. A failed draw is
rare (shm allocation refused under fd pressure) but real. Retry on the next tick,
bounded (a few consecutive failures, then quiet, as scoot's own present-skip retry
does), with a test that forces a refused buffer and shows the clock recovers.

Done in the follow-ups PR (#335): `Output::failed` is now the frame plus a
consecutive-failure count, capped at 3 exactly like scoot's `present_retry`
(`crates/scoot/src/compositor/tty/present_retry.rs`: three `Arm`s, then
quiet; a shown draw resets the streak, as an issued flip does there). A
failed draw is retried on later turns — the next clock tick in the common
case — said once on stderr per streak (retries stay quiet), then not tried
again until the frame changes; an acked `configure` still gets its commit
once quiet on a mapped surface. The workspaces module (PR #334) draws
through the same `draw_failed` call in `daemon/mod.rs` `draw` — its change
reports arrive as `Update::Changed` through `Module::on_dispatch`, which
feeds the same `Scene::stale` → `plan` → draw path — so a failed workspace
redraw retries the same way with no workspaces-specific code. Tests in
`outputs/tests.rs`: a failed draw retries a few times then goes quiet even
for a stale tick, and recovers once a retry shows; a failed redraw retries
first, then commits the ack once quiet.

## 2. `TZ=EST5EDT` with no zoneinfo becomes UTC

`zone.rs` treats a POSIX `TZ` with no explicit rule and no zoneinfo file as UTC,
where glibc applies the default US rules. The code documents this choice. Decide:
keep it (and say so in `cli.md`), or apply glibc's default rules.

Decided in the follow-ups PR (#335): apply glibc's rules. `Tz::posix`
retries a ruleless DST name with `,M3.2.0,M11.1.0` appended, after checking
`date` with an empty `TZDIR` shows byte-identical summers to the explicit
rule (March's second Sunday to November's first, 02:00 both ends). Showing
UTC while the whole system shows EDT would be a silent wrong time; matching
glibc is ten lines. Zone-file footers stay strict (an unparsable footer
still means the last type in force — zic never writes a ruleless one), and
`cli.md` says the choice in one sentence. Tests: bare `EST5EDT` equals the
explicit rule with the right 2026 boundaries, a ruleless footer stays EST
all year, and `zone::load` of `EST5EDT` with no zone file is silent EDT/EST.

## 3. The fuzz crate is never compiled in CI

A refactor of `format.rs` or `tzif.rs` can break the fuzz crate's `#[path]`
includes silently. scootbg's fuzz crate is the same. Add a cheap
`cargo check` of both fuzz crates to the path-filtered jobs (no fuzzing, just
compiling), so the includes cannot rot.

**Done** by [testing-and-ci](resolved/testing-and-ci-done.md): the scootbar
job builds and runs both of scootbar's targets for a fixed budget, and the
scootbg job runs `cargo check` of its fuzz crate. The record shows a change
that compiles in scootbar and fails only there.

Verified 2026-09-29 against `.github/workflows/ci.yml`: the `scootbar`
job's "Fuzz targets, a fixed budget" step builds and runs both targets
(`format` 1,000,000 runs, `tzif` 5,000,000, seed 1) after `cargo fetch
--locked` of the fuzz workspace, and `docs/scootbar/testing.md` documents
it. Nothing to do.

## 4. "A warm tick allocates nothing" is read, not measured

The reviewer verified it from the code. Pin it with a counting-allocator test
(a warm tick, a cache-hit glyph run, zero allocations) so a later change cannot
add one.

Done in the follow-ups PR (#335): `scootbg-mem::count` is a counting global
allocator forwarding everything to `System` (thread-local arming, so
concurrent neighbours under `cargo test` cannot pollute it), with its own
`tests/count.rs` showing empty counts 0 and a `vec!` counts something; and
`modules/clock/tests.rs::a_warm_tick_allocates_nothing` arms it around one
full loop turn (refresh, measure, paint, damage) after three warming turns
and asserts zero. Sensitivity: a `format!` probe in the window fails with
`left: 1, right: 0`. The counter lives in `scootbg-mem` because scootbar is
`#![forbid(unsafe_code)]` and a `GlobalAlloc` impl needs `unsafe`; the
shipped binary is untouched (the allocator is `#[cfg(test)]`). No behavior
change: read-only measurement.

## 5. Tests that have never run in CI

The opt-in real-clock-step and DST test (`SCOOTBAR_TEST_SET_CLOCK=1`) has never
been run by anyone: setting the system clock was denied. Find a way to run it
(a container with `CAP_SYS_TIME`, or a `libfaketime`-style harness) or state why
the unit tests cover it.

Genuinely unrunnable in this container (2026-09-29): `capsh` shows the IAB
denies `cap_sys_time`, `clock_settime(CLOCK_REALTIME)` fails with `EPERM`
(errno 1), `unshare --time` fails with `EINVAL` (no time namespaces), and no
`libfaketime` exists on the box (only Go runtime testdata, unrelated). A
time namespace would not help anyway: it virtualizes monotonic/boot time,
not the realtime clock this test steps. The test stays opt-in for a
disposable root box. What the unit tests do cover of the same path: the
step arithmetic (`a_step_shows_at_once_and_rearms_for_the_new_time`), both
DST boundaries on the minute (`the_clock_falls_back/springs_forward…`),
the arm-for-the-next-boundary timer (`a_minute_clock_is_armed…`), the real
timer firing (`the_timer_fires…`, `an_error_or_hang_up…`), and zone reload
(`a_changed_zone_file_is_read_again`). What stays uncovered without the
opt-in run is only the end-to-end latency: a real step showing on screen
within seconds through `TFD_TIMER_CANCEL_ON_SET`.

## Done when

Each item is fixed, or explicitly declined here with the reason.
