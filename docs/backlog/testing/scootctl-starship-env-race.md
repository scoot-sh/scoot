---
title: "scootctl look wiring tests race on STARSHIP_CONFIG (flaked #672 CI)"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# scootctl look wiring tests race on STARSHIP_CONFIG (flaked #672 CI)

Filed 2026-10-18. Serves **daily-drive (a flaky suite hides real failures)**.

## The gap

`look::wiring::tests::missing_wiring_names_the_exact_fix_with_real_paths` failed once in CI's `cargo test` (one process) step on PR #672, which does not touch scootctl; a rerun passed. `starship_config_env_counts_as_wired` (`crates/scootctl/src/look/wiring/tests.rs`, ~line 203) mutates the process environment (`STARSHIP_CONFIG`) while sibling tests read it, and `cargo test` runs them concurrently in one process. nextest hides it by isolating processes.

## What to do

Make the env read injectable (pass the value in, as the other wiring inputs are) or serialise the env-touching tests behind one lock; add a test that fails when the lock or injection is removed. Audit the rest of scootctl's tests for `set_var`/`remove_var`.

## Not in this ticket

Changing how `STARSHIP_CONFIG` is interpreted for users.
