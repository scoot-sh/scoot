---
title: "Unbounded `accept()` in the `msg_broken_pipe` fixture turns a transient connect failure into an infinite suite hang — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Unbounded `accept()` in the `msg_broken_pipe` fixture turns a transient connect failure into an infinite suite hang — DONE

RESOLVED 2026-09-20 (test-only + one config stanza; no production code
changed). Both fix shapes landed, layered: the fixture names the cause,
the nextest knob catches everything else.

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #168, where
`scootctl::msg_broken_pipe a_full_read_is_unchanged` stuck >840s in a
parallel nextest run (0.00s solo, green rerun — transient, not caused by
that PR, which touches nothing in this crate).

## Mechanism (verified in source by the reviewer, not reproduced)

`serve_once` (`crates/scootctl/tests/msg_broken_pipe.rs:47-65`) sets 10s
read/write timeouts on the *accepted* socket, but `listener.accept()`
itself (line 50) has no timeout, `Command::output()` (lines 142-147) has
no timeout, and there is no `.config/nextest.toml` slow-timeout anywhere
in the tree. If the child ever fails to connect, the server thread blocks
in `accept()` forever and `server.join()` (line 149) hangs the test
forever — a hang, not a failure. Concrete scenario: under a loaded
parallel run a `scootctl` spawn/connect hiccups once → this test never
fails, never passes, and wedges the whole nextest run until someone kills
it. That is worse than a flake: a flake goes red and gets noticed.

## Fix shapes (either, not both)

- An accept timeout (or non-blocking accept + deadline) in the fixture, so
  a connect failure becomes a loud test failure; or
- a `slow-timeout` in a `.config/nextest.toml`, so any future infinite
  hang anywhere fails loudly instead of wedging the run (this would also
  have caught the 10s-`PATIENCE` flood timeouts faster — those at least go
  red on their own).

Prefer the fixture timeout (it names the failure), and consider the
nextest knob as a backstop in the same PR if it is one config stanza.

## Out of scope

Production code (the `msg` client path is fine — this is the test
fixture), any change to PR #168's scope.

## Resolution

`serve_once` now accepts through `accept_with_deadline` (non-blocking
listener + 10ms poll + 30s deadline) instead of a bare blocking
`accept()`. On expiry the server thread panics with `TimedOut: "fake IPC
server: no client connected within 30s — the scootctl child failed to
spawn or connect"`, and the two `serve_once` call sites join through
`join_server`, which re-raises with `resume_unwind` — a bare
`join().unwrap()` would have discarded the message behind `Any { .. }`
and turned the named cause back into an anonymous join failure.

`.config/nextest.toml` (new — none existed tree-wide) carries the
backstop as one stanza: `slow-timeout = { period = "60s",
terminate-after = 2 }`. A test running past two slow periods is
terminated and fails; deliberately no `retries`, so a hang stays red.
CI picks it up with no workflow change: `--config-file` defaults to
`workspace-root/.config/nextest.toml` and CI runs `cargo nextest run
--workspace` from the repo (workspace) root.

Deliberately unchanged: `Command::output()` / `child.wait()` still have
no timeout of their own — a post-connect wedge (child connects, then
never answers) is now the backstop's job (terminate at ~120s), not the
fixture's. The pre-existing 10s read/write timeouts already bound the
server side of that shape.

## Sizing (measured 2026-09-20, dev VM)

- Fixture `ACCEPT_TIMEOUT = 30s`: the connect-inclusive fixture tests
  take 0.019–0.113s inside a full 1212-test parallel run (0.011–0.090s
  solo) — >250x margin against a flaky-fast timeout, ~28x faster than
  the observed >840s wedge. 3x the existing 10s I/O timeouts, which
  cover strictly less work than spawn + exec + connect.
- Nextest `period = 60s, terminate-after = 2`: slowest legitimate test
  in the full run is 7.72s (a `scoot-core` fuzz invariant),
  next-slowest 1.23s; the largest designed in-suite wait is the 10s
  `PATIENCE`. So 60s is ~8x the measured max (6x the largest designed
  wait) before a test is even *marked* slow, and termination at ~120s is
  ~15x / 12x — margin for slower CI hardware and fuzz variance, while a
  true wedge fails ~7x faster than the observed stick.

## Evidence (recorded, dev VM over the 9p mount unless noted)

- Before (Mac-side scratch, `rustc`, exact old shape — blocking
  `accept()`, no client): still pending after 5s, `HUNG as predicted`.
- After, induced connect failure (temporary test in the fixture file,
  `serve_once` + no client, removed before commit): `FAILED … finished
  in 30.01s` with `TimedOut … "fake IPC server: no client connected
  within 30s — the scootctl child failed to spawn or connect"` —
  fails loud in N seconds, names the cause.
- After, knob fire proof (temporary 120s-sleep test + `--config-file`
  override at `period = 5s, terminate-after = 1`, both removed):
  `TIMEOUT [ 5.004s] … 0 passed, 1 timed out` — the backstop
  terminates and fails, never masks.
- New committed regression test
  `accept_timeout_fires_without_a_client` (2s deadline, no client can
  ever connect so the lower bound is load-proof): PASS 2.015s nextest,
  inside the 2.00s `cargo test` binary.
- Full greens: `cargo nextest run --workspace` 1212 passed / 4 skipped
  (32.111s); `cargo test --workspace` all suites ok, 0 failed;
  `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo fmt --check --all` clean.
- No VM hardware beyond test runs (fixture-only change; nothing
  backend-specific to exercise).
- `git diff --name-only`: `crates/scootctl/tests/msg_broken_pipe.rs`,
  `.config/nextest.toml`, this record, `docs/backlog/README.md`,
  `ROADMAP.md` — zero production files.
