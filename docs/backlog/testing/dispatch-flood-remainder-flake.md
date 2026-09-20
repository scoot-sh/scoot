---
title: "Same fd-pressure flake class in two more dispatch flood tests (`immed`, second-client)"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Same fd-pressure flake class in two more dispatch flood tests

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #168, which hardened
two dispatch floods against fd-pressure kills and explicitly scoped these
two out. The reviewer confirmed both are real and correctly left out;
this entry is the filing.

## `a_dmabuf_immed_…` (`dispatch/tests.rs:1244`)

Still the old shape (in-closure lock, code+interface-only assert, no
headroom) with the byte-identical fill as the fixed `create` half, and it
reproduced red under `prlimit 650` in PR #168's suite table. Same two-line
treatment applies: headroom check + cause-pinning by message.

## `a_second_client_buffers_…` (`dispatch/tests.rs:1071`)

Asserts the fill *succeeds*, so it wants the headroom half only — no
message pin (there is no kill to discriminate). Reproduced red under
`prlimit 650` the same way (fill refused mid-way → 10s deadline).

## Constraint for whoever takes this (from the same review)

Note 4 on PR #168: residual in-closure lock acquisitions remain at
`tests.rs:984, 1013, 1073, 1246`, and the fixture strands an in-closure
panic on its 10s dispatch deadline (`drive`, lines 254-269 — a closure
panic never sets `finished`). Any headroom check here must go on the test
thread per PR #168's precedent, not copy the neighboring in-closure shape.

## Out of scope

Production behavior (the pressure refusal is correct — test-only, like the
siblings), CI/ulimit changes. See the sibling records for the shared
vocabulary (`FD_FLOOD_LOCK`, `TARGET_SOFT` 4096, `NEIGHBOUR_SLACK` 256,
raise-only-then-verify, loud-panic-never-skip):
`../resolved/icon-buffer-budget-fd-pressure-flake-done.md`,
`../resolved/dispatch-flood-fd-pressure-flake-done.md`.
