---
title: "Detached cargo test hangs a_signal_kills_the_daemon: inherited SIG_IGN on SIGINT"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Detached cargo test hangs a_signal_kills_the_daemon: inherited SIG_IGN on SIGINT

Filed 2026-10-08. Serves **computer use**: the daemon's shutdown path is
what an agent driving scootbg over IPC depends on when a session ends.

## The gap

`a_signal_kills_the_daemon…` hangs (20 s timeout) when the test session
itself runs detached — inherited `SIG_IGN` on SIGINT, not a product bug.
Mechanism, nailed on one box and one tree (review of PR #514):

- The daemon installs no signal handlers and relies on default
  dispositions (`crates/scootbg/src/daemon/mod.rs:31`).
- `cargo test` (unlike nextest) does not sanitize signal dispositions for
  test children, so a backgrounded/detached session (setsid/nohup/`&`
  with job control off → bash ignores SIGINT/SIGQUIT → inherited across
  exec) leaves the daemon ignoring SIGINT, and the test's INT iteration
  hits the 20 s timeout.
- Evidence: detached `cargo test` fails (3×, 20.06 s, loads 1.1–2.1);
  detached `cargo nextest` passes (0.118 s); foreground `cargo test`
  passes the full suite even at load ~8; `/proc` showed the hung daemon
  with `SigIgn: …1006` (SIGINT+SIGQUIT+SIGPIPE ignored).
- CI runs the test in the foreground and is green; the delta that found
  this touches no signal/spawn/disposition code.

## What to do

Pick one, in one small PR with a test that fails detached before it:

- Reset SIGINT/SIGQUIT to `SIG_DFL` at daemon startup (a daemon
  backgrounded from a script ignoring Ctrl-C is arguably a real wart),
  or
- reset the dispositions in the test harness's spawn path (`pre_exec` or
  equivalent), so the test controls its own premise.

Edge cases: SIGPIPE stays ignored (Rust runtime behavior for pipes);
SIGTERM/SIGHUP handling is unchanged; the fix must not alter foreground
behavior (the suite stays green both runners, foreground and detached).

## Not in this ticket

The slideshow/transition work that surfaced this (PR #514); any broader
signal-handling design for the daemon.

## Resolved 2026-10-08 (PR #540)

Harness fix, the ticket's second bullet (test controls its own premise):
`Session::scootbg()` (`crates/scootbg/tests/common/mod.rs`) resets
SIGHUP/SIGINT/SIGQUIT to `SIG_DFL` in the child via `pre_exec` before exec,
covering every session spawn including daemons started through
`apply-config`. Direct `signal(2)` without the `libc` crate; SIGPIPE stays
ignored, SIGTERM untouched. SIGHUP goes beyond the ticket's SIGINT/SIGQUIT
because `nohup`/detached also ignores HUP and the brief requires a
`nohup`/detached verification (an INT/QUIT-only fix still hung on the HUP
iteration under `nohup`). No product change, foreground behavior unchanged,
trivially reversible.

Revert-run-restore on the Asahi M2 (loads recorded in the PR report):
before, detached `cargo test` (forced `SIG_IGN` via `trap "" INT QUIT`)
FAILED in 20.06 s ("the daemon did not exit within 20s"); after, the same
run passes in 0.15 s, foreground `cargo test` in 0.12 s, detached and
foreground `nextest` in 0.123 s / 0.121 s. `/proc` showed the fixed daemon
with `SigIgn: ...1001` (only HUP+PIPE from `nohup`/runtime) instead of
ignoring INT/QUIT. Full `nextest -p scootbg -p scootbg-mem --no-fail-fast`:
638 passed, 2 failed (rotation load flakes, both pass alone), 3 skipped;
`clippy -D warnings` clean, `fmt --check` clean, `cargo deny check` ok,
`backlog check` the 3 pre-existing problems only. Release `scootbg`
1,839,904 B file / 1,759,705 B `.text`: test-only, no product bytes changed.
