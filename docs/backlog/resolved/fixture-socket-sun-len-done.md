---
title: "Test socket paths overflow macOS `SUN_LEN` under a long `$TMPDIR` — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Test socket paths overflow macOS `SUN_LEN` under a long `$TMPDIR` — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). Filed
2026-09-20 from `scoot-reviewer`'s pass on PR #180 (verified live on the
dev Mac, pre-existing on unmodified `main`, unrelated to that PR).

## Mechanism (as filed; reproduced on this Mac pre-fix)

`accept_timeout_fires_without_a_client` (and by construction the whole
`msg_broken_pipe.rs` fixture family) built its socket path as
`$TMPDIR/scoot-epipe-test-{pid}-{name}-{nanos}.sock`. Measured on this
dev Mac: `$TMPDIR` is 49 bytes
(`/var/folders/kx/mc82cbps4cj18f9xtdg4scc00000gn/T/`), the old filename
62 bytes (5-digit pid, 14-char `accept-timeout` name, 19-digit nanos) —
111 total, past macOS's 104-byte `SUN_LEN`. `UnixListener::bind` fails
with `InvalidInput: "path must be shorter than SUN_LEN"`. Linux allows
108 and the dev VM's `$TMPDIR` is short, so the suite stayed green where
it runs.

## Resolution

The ticket's shorten shape, exactly as filed: `socket_path` now builds
`scoot-ep-{pid}-{nanos-lo32-hex}-{tag}.sock` — shorter prefix, nanos as
8 fixed hex digits, per-test tag (`large`, `full`, `acc` for the
accept-timeout test). New filename lengths: 34 / 33 / 32 chars; totals
against this Mac's 49-byte `$TMPDIR`: 83 / 82 / 81, i.e. 21+ bytes of
headroom under the 104 limit. Worst-case audit: a 7-digit Linux pid
with the longest tag (`large`) is still a 36-char filename, fitting a
68-char `$TMPDIR` on macOS. The why (limit math, uniqueness argument,
tag mapping) is written down next to the code in `socket_path`'s doc
comment, not just here. `std` only — the "hash" is a plain low-32-bit
truncation formatted hex, no new deps for a test fixture.

Deliberately unchanged: everything else in the fixture (30s
`ACCEPT_TIMEOUT`, bind-before-spawn, 2s regression test, both reply
builders), all production code, CI.

## Bug-bash (sized to the change)

- Uniqueness after shortening: pid (across processes — nextest
  isolates each test in its own), tag (across tests sharing one `cargo
  test` process — the three socket tests each carry a distinct one),
  nanos low-32 (across sequential re-runs, on top of the `remove_file`
  of a stale path at each call site). A residual collision still fails
  loudly at `bind(...).unwrap()` on the calling thread, never inside
  the server thread — that property is untouched.
- Nanos truncation (low 32 bits cycle ~4.3s): harmless here, since
  same-process repeats are separated by the tag and cross-process
  repeats by the pid; the truncation only guards stale-path re-runs,
  where `remove_file` already does the real work.
- Debuggability: the tags are documented in the doc comment, so a
  stray `scoot-ep-*-acc.sock` still names the accept-timeout test as
  its owner.
- Linux unaffected: the construction is identical on both platforms
  (no `cfg`), so the Linux greens below prove the same code path;
  Linux's 108-byte limit was never the binding constraint anyway.
- Siblings checked: the other `temp_dir()` fixtures in the tree
  (`child_reaper/tests.rs`, `activation/tests/spawn.rs`,
  `activation/tests/session_env.rs`) write regular probe files, not
  Unix sockets — `SUN_LEN` does not apply. Out of scope, left alone.

## Evidence (recorded; branch `backlog/fixture-sun-len` atop `80d0c45`)

CI-invisibility, stated per the ticket: the macOS CI job runs only
`cargo check --workspace --all-targets` (verified in
`.github/workflows/ci.yml`, `macos` job) and never executes tests, so
all of the following Mac evidence is local-only — no CI coverage is
claimed.

- Mac pre-fix: `cargo test -p scootctl --test msg_broken_pipe
  accept_timeout_fires_without_a_client` → **FAILED** with `Error {
  kind: InvalidInput, message: "path must be shorter than SUN_LEN" }`
  at `msg_broken_pipe.rs:251` (`UnixListener::bind(&path).unwrap()`).
- Mac post-fix: `cargo test -p scootctl --test msg_broken_pipe` →
  **4 passed, 0 failed** (2.01s).
- `cargo fmt --check -p scootctl` (Mac): clean. `cargo clippy -p
  scootctl --all-targets` (Mac): clean.
- Linux dev VM (working tree via the 9p mount, same uncommitted
  state): `cargo nextest run --workspace` → **1212 passed, 4
  skipped** (32.247s); `cargo clippy --workspace --all-targets -- -D
  warnings` → clean; `cargo fmt --check --all` → clean.
- No VM hardware beyond test runs (fixture-only change; nothing
  backend-specific to exercise).
- `git diff --name-only`: `crates/scootctl/tests/msg_broken_pipe.rs`,
  this record, `docs/backlog/README.md`, `ROADMAP.md` — zero
  production files.
