---
title: "Bind-before-spawn in the `msg_broken_pipe` fixture (residual bind-vs-connect race)"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Bind-before-spawn in the `msg_broken_pipe` fixture

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #179, which bounded
the `accept()` wedge with a 30s deadline and recommended this as the
follow-up rather than scope for that PR.

## Mechanism (observed live 4× by the reviewer, ~18% of rapid isolation reruns, 0 in full-workspace runs)

`serve_once` (`crates/scootctl/tests/msg_broken_pipe.rs`) spawns the
server thread, which calls `UnixListener::bind(&path)` inside the thread,
while the main thread immediately spawns the `scootctl` child — no
happens-before edge between `bind()` and the child's `connect()`. When
the server thread loses the race, the child gets ENOENT and exits 1 in
milliseconds; the server thread polls `WouldBlock` for the full 30s, then
the test goes red with the (accurate, actionable) `TimedOut` message.
Pre-#179 the identical sequence wedged the suite forever, which is almost
certainly what produced the original >840s stick — so this is strictly an
improvement, and the 30s reds themselves prove the deadline bites.

## Fix (~5 lines)

Bind the listener on the calling thread *before* spawning the server
thread/child, passing the bound listener in. Program-order `bind` →
child-spawn is a complete happens-before fix. Alternative: connect-retry
in the fixture. Either closes it; prefer bind-before-spawn (no retry
timing to size).

## Proof shape

~22 rapid isolation reruns of the `msg_broken_pipe` suite (the shape that
showed 4/22) going green, plus the standard full cheap set. The committed
2s no-client regression test still exercises the deadline path unchanged.

## Out of scope

Production code (fixture only), any other test, CI changes.
