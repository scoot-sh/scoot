---
title: "Unbounded `accept()` in the `msg_broken_pipe` fixture turns a transient connect failure into an infinite suite hang"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# Unbounded `accept()` in the `msg_broken_pipe` fixture turns a transient connect failure into an infinite suite hang

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
