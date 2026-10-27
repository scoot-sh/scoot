---
title: "scootctl look wiring tests race on STARSHIP_CONFIG (flaked #672 CI)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# scootctl look wiring tests race on STARSHIP_CONFIG (flaked #672 CI)

Filed 2026-10-18. Serves **daily-drive (a flaky suite hides real failures)**.

## The gap

`look::wiring::tests::missing_wiring_names_the_exact_fix_with_real_paths` failed once in CI's `cargo test` (one process) step on PR #672, which does not touch scootctl; a rerun passed. `starship_config_env_counts_as_wired` (`crates/scootctl/src/look/wiring/tests.rs`, ~line 203) mutates the process environment (`STARSHIP_CONFIG`) while sibling tests read it, and `cargo test` runs them concurrently in one process. nextest hides it by isolating processes.

## What to do

Make the env read injectable (pass the value in, as the other wiring inputs are) or serialise the env-touching tests behind one lock; add a test that fails when the lock or injection is removed. Audit the rest of scootctl's tests for `set_var`/`remove_var`.

## Not in this ticket

Changing how `STARSHIP_CONFIG` is interpreted for users.

## Second sighting (2026-10-20)

Failed again on the Linux (compositor) job on the head of #699 (no Rust changed in that PR) and passed on re-run: `missing_wiring_names_the_exact_fix_with_real_paths` under one-process `cargo test`. Two hits in three days means the fix is due.

## Resolution (2026-10-27)

The injection fix landed in #711 (`fix(scootctl): thread the starship config through the wiring input`): `WiringInput::starship_config` threads the location like every other path, `resolve_input` reads `STARSHIP_CONFIG` once in production, tests pin it explicitly and never touch the environment; `starship_config_injection_counts_as_wired` fails if the field is removed (regression pin). This PR verifies and audits, no new production code.

Evidence: before — CI failures on PR #672 and #699 (`missing_wiring_names_the_exact_fix_with_real_paths` under one-process `cargo test`, pass on re-run). After — dev VM (`ssh -p 2223`, `/dev/shm/wt-bl5`, `CARGO_INCREMENTAL=0`, `nix develop path:/dev/shm/wt-bl5 -c cargo ...`): `cargo test -p scootctl --lib look::wiring` 50/50 green. Audit 2026-10-27: `wiring/` has no `set_var`/`remove_var` (only comments); `tests/look_help_docs.rs` mutates env only in the re-exec'd child (`SCOOT_LOOK_HELP_DOCS_CHILD`, separate process); `tests/look_leaves_cli.rs` serializes `PATH` behind `PATH_LOCK`; `cycle/tests.rs` and `apply/tests.rs` read env but never mutate. No remaining shared-process env race.
