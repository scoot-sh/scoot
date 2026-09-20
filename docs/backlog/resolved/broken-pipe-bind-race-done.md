---
title: "Bind-before-spawn in the `msg_broken_pipe` fixture (residual bind-vs-connect race) — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Bind-before-spawn in the `msg_broken_pipe` fixture — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). Filed
2026-09-20 from `scoot-reviewer`'s pass on PR #179, which bounded the
`accept()` wedge with a 30s deadline and recommended this as the
follow-up rather than scope for that PR.

## Mechanism (as filed; reproduced live 1× in the before-shape control below)

`serve_once` (`crates/scootctl/tests/msg_broken_pipe.rs`) spawned the
server thread, which called `UnixListener::bind(&path)` inside the
thread, while the main thread immediately spawned the `scootctl` child
— no happens-before edge between `bind()` and the child's `connect()`.
When the server thread lost the race, the child got ENOENT and exited 1
in milliseconds; the server thread polled `WouldBlock` for the full
30s, then the test went red with the (accurate, actionable) `TimedOut`
message. Pre-#179 the identical sequence wedged the suite forever,
which is almost certainly what produced the original >840s stick.

## Resolution

Bind-before-spawn, the ticket's preferred shape (no retry timing to
size, so the connect-retry alternative was never needed): `serve_once`
now takes an already-bound `UnixListener` instead of a path, and both
call sites (`closed_stdout_on_a_large_reply_exits_quietly`,
`a_full_read_is_unchanged`) bind on the calling thread before the
server thread or the child exists. Program-order `bind` → spawn is the
complete happens-before fix. The why is written down next to the code
(`serve_once` doc comment + per-call-site notes), not just here.

Deliberately unchanged: the 30s `ACCEPT_TIMEOUT`, the nextest backstop,
and the committed 2s no-client regression test
(`accept_timeout_fires_without_a_client`), which still exercises the
deadline path exactly as before.

## Bug-bash (sized to the change)

- Listener ownership across the thread boundary: the bound listener
  moves into the server thread and drops at thread end, exactly the old
  lifecycle minus the in-thread `bind` — no fd held longer, no leak.
- Unique per-test socket paths (`pid` + nanos) are untouched, so
  still unique when bound earlier; a collision now fails on the calling
  thread via `bind(...).unwrap()` — loud and immediate, never a hang,
  and attributed to the right test instead of surfacing through the
  thread join.
- The 2s regression test still fires its deadline: every post-fix
  isolation run below takes ~2.0s, i.e. the deadline path is exercised,
  not skipped.

## Evidence (recorded, dev VM over the 9p mount unless noted)

Rerun method (both shapes): the compiled `msg_broken_pipe` test binary
run directly, 22× back-to-back in isolation — the shape that showed
4/22 to the reviewer.

- Before-shape control (fixture temporarily restored to `main`'s
  thread-internal `bind`, rebuilt — binary newer than source verified
  before running): **21/22 green, 1 race-red**: run 11,
  `a_full_read_is_unchanged` FAILED in 30.01s with `TimedOut: "fake
  IPC server: no client connected within 30s — the scootctl child
  failed to spawn or connect"` — the exact ticket mechanism, live on
  this machine. (Reviewer saw 4/22; the rate varies with load. One
  reproduction is what the control needs: the race fires here, so the
  post-fix green is not vacuous.)
- After (fix restored, rebuilt): **22/22 green**, each run ~2.00–2.01s
  (the 2s deadline test dominating, as designed).
- `cargo nextest run --workspace` (dev VM): **1212 passed, 4 skipped**
  (31.993s).
- `cargo test -p scootctl --test msg_broken_pipe` (dev VM): **4 passed,
  0 failed** (2.01s).
- `cargo clippy --workspace --all-targets -- -D warnings` (Mac): clean.
- `cargo fmt --check --all` (Mac): clean.
- No VM hardware beyond test runs (fixture-only change; nothing
  backend-specific to exercise).
- `git diff --name-only`: `crates/scootctl/tests/msg_broken_pipe.rs`,
  this record, `docs/backlog/README.md`, `ROADMAP.md` — zero
  production files.

Mac-side note (pre-existing, unrelated, left alone per scope): on this
Mac the suite's `accept_timeout_fires_without_a_client` fails with
`InvalidInput: "path must be shorter than SUN_LEN"` — `$TMPDIR`
(`/var/folders/...`, 50 chars) plus the socket filename (62 chars)
exceeds macOS's 104-byte `SUN_LEN`. Reproduced on unmodified `main`
via a throwaway worktree, so it predates this change; the fixture,
paths, and that test are untouched by it. All proof runs above are on
the dev VM, where `/tmp` is short and the full suite goes green.
