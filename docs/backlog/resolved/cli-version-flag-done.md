---
title: "No `scoot --version`: a build can only be identified by starting it — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No `scoot --version`: a build can only be identified by starting it — DONE

Requested as gh issue #176. Resolved 2026-09-21 as decided, with the
ticket's shape intact (PR #195): `scoot --version` and `scootctl --version` print
`scoot 0.1.0 (ipc protocol 3)` — the same `env!("CARGO_PKG_VERSION")`
the IPC `version` reply reads (agree by construction) plus the IPC
protocol number — and exit 0 with no compositor running.

## What landed

- **`scoot --version`** (new `cli::Command::Version` variant, first-arg
  flag like `--help`/`--print-default-config`): prints the shared
  `scootctl::version_string()` through `scootctl::output::print_line`, so
  it inherits the client output contract — a closed stdout
  (`| head -c0`) is a quiet exit 0, proven live on Linux, not a panic.
  Runs on every platform (no compositor started), like `--help`.
- **`scootctl --version`** (new `scootctl::Command::Version`): the same
  line, byte for byte, from the same helper — a packaging check can
  compare the two binaries' outputs directly. Answered locally, never a
  request, so it needs no socket.
- **The helper** (`scootctl::cli::version_string`, re-exported at crate
  root): `format!("scoot {} (ipc protocol {})",
  env!("CARGO_PKG_VERSION"), scoot_ipc::PROTOCOL_VERSION)` — derived from
  the two live constants, never a duplicated literal, so the string cannot
  drift when either moves. Both crates read `version.workspace`, so both
  `env!`s name the same version.
- **Nine tests**: five on the `scootctl` side (parses to its own command,
  first-arg-like-help in both directions, the bare-word-stays-remote pin,
  the recomputed-from-constants no-drift pin, the `--help` usage-line
  pin) and four on the `scoot` side (parses to its own command with
  trailing args ignored, first-arg-only pin, usage-line pin, the
  this-binary's-version-and-live-protocol pin).
- **Docs**: `--help` usage lines on both binaries,
  `docs/configuration.md` usage block + flag-table row, `docs/ipc.md`
  `version`-row distinction (remote request vs local flags), README
  Running one-liner.

## Decisions

1. **`scootctl --version` (flag), not `scootctl version`** (implementer
   decision, per the ticket's explicit delegation): the bare `version`
   word already means the IPC `Request::Version`, answered by the
   compositor over the socket. Giving one spelling two transports — local
   when no session is up, remote when one happens to be — would make its
   failure mode depend on whether a compositor is running. Pinned by
   `a_bare_version_word_stays_the_ipc_request` and proven live:
   `scootctl version` with no session exits 1 (`No such file or
   directory`), `scootctl --version` exits 0.
2. **Both binaries print `scoot ...`, byte-identically, from one helper**
   (implementer decision): the line names the suite, not the binary —
   the versions move in lockstep via `version.workspace` — so equality of
   the two outputs is itself a check. Proven live (`IDENTICAL`).
3. **`--help` semantics, not new ones** (per ticket): first arg only,
   trailing args ignored (`--version --headless` still answers);
   behind a backend flag it errors (`--headless --version` is
   `Unknown`, pinned by test). No `-V` short flag: the ticket asked for
   `--version`, and the codebase favors a small surface.
4. **No wire change, no `PROTOCOL_VERSION` bump** (per ticket scope):
   read-only addition; the `version` reply is untouched.

## Evidence

- Pre-fix (branch base `f5b6b90`, Mac binaries): both answered
  `unknown argument '--version' (try --help)`, exit 1.
- `cargo nextest run --workspace` (Mac): 162 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --check --all`: clean.
- `scripts/smoke-test.sh` on the dev VM (binaries built from this
  branch at `/var/cargo-target`): green end to end — the new match arms
  sit in the shared first-arg dispatch without touching any existing arm
  or the `message()`/`compositor()` paths, and the smoke run confirms no
  existing argv behavior moved.
- Live (Mac + Linux binaries from this branch):
  `scoot 0.1.0 (ipc protocol 3)`, exit 0, both binaries, byte-identical
  (29 bytes, one trailing `\n`); closed-stdout (`>&-` on macOS,
  `| head -c0` on Linux) exit 0 on both; `--version` with trailing args
  answers; `--headless --version` errors; `--Version` errors;
  `scoot msg --version` still `Unknown`.
- Benchmark: n/a — a cold one-shot flag on process startup, no hot path
  touched (no allocation, IPC, render-loop, or input-dispatch code).

## Left out (with why)

- **`scoot msg version` learns nothing local**: still the remote request
  by decision 1 — `msg` is the alias for the socket client, and a local
  answer there would reintroduce the two-transports-one-spelling split.
- **README carries one line, not a section**: the flag needs no
  workflow; the `--help` pointers plus the Running one-liner cover it.
- **`version` reply fields stay undocumented in `docs/ipc.md`**: the
  "What the replies carry" section never documented them, and that gap
  predates this ticket — out of scope.
