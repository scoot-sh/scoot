---
title: "SIGHUP trigger for config reload (second trigger alongside IPC) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# SIGHUP trigger for config reload (second trigger alongside IPC) — RESOLVED

## What it said

Follow-up to `config-reload-done.md`: a `SIGHUP` handler driving the same
shared reload path as the IPC request (validate-before-apply,
applied/refused semantics, TTY guard ride along), applied/refused summary to
the log (no reply channel), default terminate disposition fully replaced.
inotify stays out. `scootctl` gets no handler.

## What landed

New `crates/scoot/src/compositor/sighup.rs` (+ `sighup/tests.rs`, 10 tests),
wired in `compositor/mod.rs::run` next to the child reaper's install:

- **Mechanism mirrors the reaper**: `sigaction` handler writing one counter
  increment to an eventfd the loop watches; the loop callback drains the wake
  and calls the shared `State::reload()`, dropping the `Response` (no reply
  channel). `reload()`'s own `info!("config reloaded", applied, refused)` and
  `error!("config reload failed")` are the operator-visible summary — no
  second log line, no forked path. `reload.rs` itself is untouched (shared
  path unmodified; the existing suite is green unmodified).
- **Composition with SIGCHLD by construction**: separate signal, separate
  `sigaction`, separate eventfd, separate calloop source — no shared state,
  so no interference is representable, not merely unobserved. Pinned live by
  `sighup_and_sigchld_do_not_interfere` (real exiting child + real HUP
  together: reaped *and* reloaded).
- **Reentrancy**: single-threaded loop; HUP mid-reload leaves a nonzero
  eventfd counter and the level trigger fires again. Rapid HUP+HUP is
  coalesced (one reload) or sequential (second sees the first's results, two
  empty lists) — both safe, pinned by
  `rapid_hup_hup_is_safe_coalesced_or_sequential` and
  `hup_racing_an_ipc_reload_runs_sequentially_on_the_shared_path`.
- **Children keep default HUP**: caught handler resets to `SIG_DFL` on exec
  (free, unlike `SIG_IGN` which survives); nothing ever blocked (no signalfd
  mask). Pinned by `a_spawned_child_sees_default_sighup_and_an_empty_mask`
  (`SigIgn`/`SigBlk`/`SigCgt` all clear of the SIGHUP bit, sensitivity proven
  in-test by forcing `SIG_IGN` and watching the probe observe it).
- **Fail-first**: `install_replaces_the_terminate_disposition` (disposition
  read before/after) plus a forked probe proving a disconnected handler
  still means terminate. And the live test with `install` neutered aborts
  with `SIGHUP` — run once by hand, output below.
- **Lock / malformed / vanished**: `sighup_applies_while_the_session_is_locked`
  (real `ext-session-lock-v1` locker client; gap applies, lock undisturbed),
  `sighup_with_a_malformed_file_keeps_the_running_config`,
  `sighup_with_a_vanished_config_file_keeps_the_running_config` (same as the
  IPC path — same `reload_from`, by construction).
- **Docs**: `docs/configuration.md` reload section owns the trigger
  paragraph (both triggers, no-file-watching rationale, `scootctl` gets no
  handler); `README.md`'s partial-reload bullet names the HUP alternative
  (it named the trigger). `docs/ipc.md` untouched (wire doc; HUP is not
  wire). No `PROTOCOL_VERSION` change, no wire change.

Out of scope, as ticketed: inotify, client-side handling, reload semantics
(all refused/applied behavior is the shared path's, unchanged).

Benchmark: n/a — signal-cold path. The handler is one atomic load plus an
8-byte `write`; the loop work is one reload per HUP, the same cost as one
IPC reload, on a trigger an operator sends by hand.

## Evidence (record, not narrative)

Branch `backlog/reload-sighup`, base `main` at `e427fd7`.

- `cargo nextest run --workspace` (dev VM, 2026-09-21): **1327 passed,
  6 skipped, 0 failed** — includes the 10 new
  `compositor::sighup::tests::*` and the unmodified reload suite.
- `cargo clippy --workspace --all-targets -- -D warnings` (dev VM): clean.
- `cargo fmt --check --all`: clean.
- `scripts/smoke-test.sh` (dev VM, `SMOKE_PREFIX=/tmp/smoke-sighup`): 19
  `ok` lines, exit 0.
- Fail-first probe (dev VM, `install` neutered in the fixture, nextest):
  `SIGHUP [0.038s] ... (test aborted with signal 1: SIGHUP)` /
  `Summary: 1 test run: 0 passed, 1 failed`. Restored after.
- Live proof (dev VM, `--headless` session off `/tmp/hup-proof/config.toml`,
  release-shape debug binary `/var/cargo-target/debug/scoot` rebuilt from
  this branch — verified `sighup` in `strings` first; the first attempt died
  on HUP against a stale pre-branch binary, which is itself the
  disconnected-handler demonstration):
  - `gap 12→20` + `background #101014→#ff0000` + `scale→2.0`, then
    `kill -HUP`: log carries `config reloaded
    applied=["layout.gap", "appearance.background_color"]
    refused=["output.scale (startup-only: clients were told the scale at
    bind time)"]`; `windows` answers before and after; before/after
    screenshots differ (`cmp`: DIFFERENT).
  - Malformed file + HUP: log carries `config reload failed ... TOML parse
    error at line 1, column 6`; `windows` answers; screenshot byte-identical
    to the post-reload one (`cmp -s`: PIXELS-UNCHANGED).

## Left out, with why

- Nothing ticketed was left out. One adjacent non-change: `docs/ipc.md`'s
  `reload` row still describes only the request — HUP is not on the wire, so
  that page has nothing to add; the trigger paragraph lives in
  `docs/configuration.md` where both triggers are named together.
