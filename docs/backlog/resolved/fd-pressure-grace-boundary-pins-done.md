---
title: "Pin the fd-pressure grace conjunction's boundaries (>, &&, 128/129, 64/65) — RESOLVED (pin, no behavior change)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Pin the fd-pressure grace conjunction's boundaries — RESOLVED (pin, no behavior change)

Filed from PR #123 review (2026-09-18). Seven tests pinned
`Table::pressured` exhaustively, but `pressure_refusal(live, grace)` in
`dispatch.rs` (`live > grace && table-pressured`) had zero tests: no pin on
`>` vs `>=`, no pin on `&&` vs `||`. An operator flip there kills
under-grace clients during pressure — the exact catastrophe the grace
exists to prevent — with no suite failure. The shipped operator was
correct as written (review-traced at all four call sites); this pins it,
it does not fix it. No behavior change: only `cfg(test)` plus doc
comments differ from `main`.

## The 400/404 verdict

The [ceiling record](./wayland-global-fd-ceiling-done.md)'s sentence —
"Two connections sitting exactly at grace hold 2 x (128 + 64 + 1) + 14
baseline = 400 fds" — is **correct as written**: it is "two AT grace"
arithmetic, and 2 x 193 + 14 = 400. No fix to the record.

The reviewer's adjacent note is also correct and is now stated exactly in
code: `live > grace` permits the grace+1-th unit (129 buffers / 65
pools), so the permitted per-connection maximum is 2 x (129 + 65 + 1) +
14 = **404**, 4 above the "two at grace" figure — negligible, but the
pins hold it there. Stated in two places: the `fd_pressure` module doc
(400 for exactly-at-grace plus the 404 permitted maximum) and the
`pressure_refusal` doc comment in `dispatch.rs` (both operators exact,
with the 404 arithmetic).

## What shipped

- `pressure_refusal`'s pure conjunction split out as
  `pressure_refusal_for(live, grace, pressured)` (`live > grace &&
  pressured`), taking the already-observed table verdict instead of
  observing. The table half reads the test process's own fd table, which
  no in-suite test can drive to pressure without starving its siblings —
  the same construction limit PR #123's record states — so the pure
  function is what gets pinned. All four call sites (the pool claim and
  the three buffer claims: shm-pool `CreateBuffer`, dmabuf
  `Create`/`CreateImmed`, single-pixel `CreateU32RgbaBuffer`) funnel
  through this predicate with one of the two grace constants, so pinning
  the constants plus the predicate covers every site.
- Five tests in `dispatch/tests.rs`:
  - `pressure_graces_are_128_buffers_and_64_pools` — the constants
    themselves (a silent grace change moves every boundary below).
  - `at_grace_passes_even_under_pressure` — `>` not `>=`: 0, grace−1,
    grace all pass under pressure, for both graces.
  - `one_past_grace_refuses_under_pressure` — 129 buffers / 65 pools
    refuse under pressure (the first refusal, not at grace).
  - `past_grace_without_pressure_passes` — `&&` not `||`, first half:
    grace+1 and `u32::MAX` with a calm table pass.
  - `under_grace_with_pressure_passes` — `&&` not `||`, second half:
    0, 1, grace−1, grace under pressure pass (the innocent-client
    guarantee).

## Fail-first

Dev VM, debug, `dispatch::tests` scope (uncommitted working tree on top
of `1fa2ce1`):

- `>` → `>=`: exactly the 2 at-grace tests fail
  (`at_grace_passes_even_under_pressure`, `under_grace_with_pressure_passes`
  on the `live == grace` leg); restored → green.
- `&&` → `||`: 8 fail — the 3 operator pins above plus 5 pre-existing
  dispatch tests (`a_second_client_buffers_while_the_first_sits_at_the_cap`,
  `a_second_client_is_unaffected_by_the_first_clients_full_cap`,
  `a_dmabuf_create_past_a_full_budget_is_refused_by_the_shared_budget`,
  `a_dmabuf_immed_past_a_full_budget_is_refused_before_validation`,
  `destroying_pools_reopens_headroom`), the flood-canary property PR #123
  claimed, firing as designed; restored → green.

## Evidence

Full standard set on the final tree, dev VM
(`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`):
`cargo test -p flexwm`, `cargo nextest run --workspace`, `cargo clippy -p
flexwm --all-targets -- -D warnings`, `cargo fmt --check -p flexwm` (fmt
Mac-side per the 9p constraint), `scripts/smoke-test.sh` — exact outputs
in the PR report.

No `README.md` change: no user-facing surface (internal pinning only, no
config/keybinding/CLI/IPC/behavior delta). No hot-path benchmark: the
split is one extra `fn` call on the refusal path only, and under-grace
creations still short-circuit on the same single `HashMap` lookup with no
syscall — the shape is unchanged, only named.
