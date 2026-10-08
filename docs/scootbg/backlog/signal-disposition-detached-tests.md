---
title: "Detached cargo test hangs a_signal_kills_the_daemon: inherited SIG_IGN on SIGINT"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
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
