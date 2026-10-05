---
title: "Flaky network menu tests: menu file read races the child"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Flaky network menu tests: menu file read races the child

Filed 2026-10-04. Serves **daily-drive reliability** (flaky tests fail CI on
unrelated PRs, which slows every other piece of daily-drive work) and
**computer use** (the network picker is agent-driving surface; its tests must
be trustworthy).

## The gap

`a_late_page_of_the_old_dump_does_not_clear_the_refill`
(`crates/scootbar/src/modules/network/tests.rs`, merged in #428) failed CI on
PR #431 — which changes no Rust — at run 37249895370, job
`scootbar (status bar)`: line 2272
(`assert_eq!(std::fs::read_to_string(&file).unwrap(), "Alpha\nBeta\n")`
after menu 3 opens) got `left: ""`. It passed on the previous CI runs.

Likely cause: `record_menu` builds `sh -c "cat > FILE"`, so each new menu
child truncates FILE at spawn and writes when its stdin closes; the test
reads FILE right after `drive()` without waiting for that child to have
written (and possibly while the replaced child is being killed). Every test
in this file that reads a menu's file after a replacement has the same race
(`grep -n record_menu`, `read_to_string(&file)`).

## What to do

Fix the tests, not the product, unless a real product bug turns up (then fix
it with a failing-first test and say so with evidence). Replace each
immediate `read_to_string` after a menu/connect spawn with a deterministic
wait: poll until the file holds the expected content, with a bounded deadline
and a clear failure message — no blind sleeps. Prove it: loop the affected
tests under load before (show the failure rate) and after (0 failures).

## Not in this ticket

Changing `open_menu`/`open_connect` spawning or reaping; the product behavior
(replace-while-running, SIGTERM-then-SIGKILL) is pinned by the
`wait_pids`/`alive` tests and stays as is.

## Resolution

Landed as a test-only fix (no product change; `crates/scootbar/.../network/`
`mod.rs` untouched): a `wait_file` helper in `network/tests.rs` polls up to
5 s for the child's file to hold the expected content (10 ms polls, both
sides in the failure message — the same shape as the existing `wait_pids`
helper), and all fifteen immediate `read_to_string` asserts after a
menu/connect spawn go through it (five in
`a_late_page_of_the_old_dump_does_not_clear_the_refill`, five in the other
`record_menu` tests, four in the `connect-command` tests, plus the
`the_menu_is_fed_the_scan_and_reaped` replacement case). Two tests that
assert `source_count` after a spawn now wait for the file first, so the
follow-up `drive()` reaps deterministically. No product bug found: the
failure mode is exactly truncate-at-spawn vs. write-when-scheduled.

Evidence (dev VM, `CARGO_TARGET_DIR=/tmp/netflake-menurace/target`):
before — 300 iterations of the flaky test at nice 19 under spinner+fork
load: 18 failures (6%), every one `left: ""` (or the file missing at the
`unwrap`), spread over all five menu reads in the test; after — the same
300 iterations: 300 passed, with 4 iterations absorbing >150 ms scheduling
delays inside `wait_file`. Whole `popup_list` module x30 under load: 30
passed. Full scootbar matrix green (`fmt`, clippy on default / no-default /
every module alone / `popup`+each module / all-features, `nextest` default
1342 passed with `SCOOTBAR_REQUIRE_SCOOT/SWAY/DBUS_DAEMON=1` against this
tree's own scoot, `cargo test`, per-module `nextest` bins, release
2,233,056 bytes / `.text` 2,105,558 — unchanged by construction, the edited
module is `#[cfg(test)]`-gated).
