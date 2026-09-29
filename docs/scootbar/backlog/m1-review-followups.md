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

## 2. `TZ=EST5EDT` with no zoneinfo becomes UTC

`zone.rs` treats a POSIX `TZ` with no explicit rule and no zoneinfo file as UTC,
where glibc applies the default US rules. The code documents this choice. Decide:
keep it (and say so in `cli.md`), or apply glibc's default rules.

## 3. The fuzz crate is never compiled in CI

A refactor of `format.rs` or `tzif.rs` can break the fuzz crate's `#[path]`
includes silently. scootbg's fuzz crate is the same. Add a cheap
`cargo check` of both fuzz crates to the path-filtered jobs (no fuzzing, just
compiling), so the includes cannot rot.

## 4. "A warm tick allocates nothing" is read, not measured

The reviewer verified it from the code. Pin it with a counting-allocator test
(a warm tick, a cache-hit glyph run, zero allocations) so a later change cannot
add one.

## 5. Tests that have never run in CI

The opt-in real-clock-step and DST test (`SCOOTBAR_TEST_SET_CLOCK=1`) has never
been run by anyone: setting the system clock was denied. Find a way to run it
(a container with `CAP_SYS_TIME`, or a `libfaketime`-style harness) or state why
the unit tests cover it.

## Done when

Each item is fixed, or explicitly declined here with the reason.
