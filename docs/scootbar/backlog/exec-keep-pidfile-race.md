---
title: "drive_placed pidfile-vs-pipe race flakes the exec keep tests"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# drive_placed pidfile-vs-pipe race flakes the exec keep tests

Filed 2026-10-02, from the review of the battery PR (#378). Serves
**daily-drive** indirectly: it is the test harness's honesty, not a
product behavior — a flaky keep test erodes trust in the exec-reload path
every M5 module's `keeps` will rely on.

## The gap

`harness::drive_placed` (`crates/scootbar/src/modules/harness.rs`)
checks `done()` at the top of the loop and returns as soon as it holds,
without consuming one more turn of ready sources. The exec keep tests
(`crates/scootbar/src/modules/keep_tests.rs`,
`a_changed_command_format_placeholder_or_restart_key_replaces_it`)
wait on the **pidfile**, but assert on the **shown text**:

```rust
drive_placed(&mut new[0], "respawned", |_| pid_in(&pidfile) != pid);
// ...
assert_eq!(text(&new[0]), later, "{tag}: the new table's child shows");
```

The replacement child writes the pidfile *before* it prints its first
line (`echo $$ > pid; echo moved; sleep ...`), so `done()` can go true
at a loop top with `"moved"` still sitting unread in the pipe. The
assert then sees the placeholder (`"ph"`), not `"moved"` — a flake,
timing-dependent, failing the suite without any product change.

## What to do

Decide where the ordering belongs and pin it with the failing scenario
above (a test that fails when the pipe line is unconsumed at `done`):

- make `drive_placed` hand over one more turn of ready sources after
  `done()` first holds (drain, then exit), or
- wait on the shown text in the keep tests instead of the pidfile
  (`|p| text(p) == later`, as the first wait in the same test does with
  `"started"`), keeping the pidfile read as the identity check only, or
- write the pidfile after the first line in the test scripts (fragile:
  it reorders the product's evidence to suit the harness).

Edge cases: a child that never prints (the wait must still time out, as
today), a `done` that holds at entry (no source to drain — must still
exit at once), and the 10 s deadline staying the bound.

## Not in this ticket

The `on-low` hook path (battery) and the daemon's hook drain: they stage
through `take_action`, not through `drive_placed`, and are covered by
harness tests that assert staged actions directly.
