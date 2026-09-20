---
title: "Test socket paths overflow macOS `SUN_LEN` under a long `$TMPDIR`"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Test socket paths overflow macOS `SUN_LEN` under a long `$TMPDIR`

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #180 (verified live
on the dev Mac, pre-existing on unmodified `main`, unrelated to that PR).

## Mechanism

`accept_timeout_fires_without_a_client` (and by construction the whole
`msg_broken_pipe.rs` fixture family) builds its socket path as
`$TMPDIR/scoot-epipe-test-{pid}-{name}-{nanos}.sock`. On the dev Mac
`$TMPDIR` alone is 50 chars and the filename runs ~63, totaling past
macOS's 104-byte `SUN_LEN` — the test fails with `InvalidInput: "path
must be shorter than SUN_LEN"`. Linux allows 108 and the dev VM's
`$TMPDIR` is short, so the suite is green where it runs.

## Why this is low, not nothing

The suite is Linux-first (compositor tests don't compile on macOS), but
the `scootctl` tests are the portable half — the one a Mac dev can run.
 macOS CI runs only `cargo check --workspace --all-targets`, never
executes tests, so this is CI-invisible: it bites only Mac devs with long
`$TMPDIR`s running the client suite locally, and any fix would be locally
verified only (state that in the record, don't claim CI coverage).

## Fix shape

Shorten/hash the socket filename in the fixture (`scoot-epipe-test-…`
~63 chars → ~32 would fix every realistic `TMPDIR`). One fixture file,
no production code, no behavior change. Verify on the dev Mac (the long
`TMPDIR` is the test environment) plus the standard Linux suite to prove
nothing moved.
