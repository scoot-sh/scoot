---
title: "Pin the budget cause by message in the two bypass-loop cap tests (assertion-vacuous under pressure)"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Pin the budget cause by message in the two bypass-loop cap tests

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #169, which hardened
the neighboring dispatch floods and explicitly scoped these two out. Not
a flake (never reddens) — a vacuous guard, which is quieter and worse.

## Mechanism (verified in source by the reviewer)

`retaining_a_buffer_past_its_pool…` and `destroying_a_pool…`
(`dispatch/tests.rs:983-1004`, `1011-1036`) assert via
`assert_shm_protocol_error` (code+interface only). Per `dispatch.rs:817-838`,
the pressure and budget refusals on `wl_shm_pool::CreateBuffer` post the
**identical** code (`InvalidStride`) on the **identical** object — so under
a pressured table the kill lands at ~grace-129 with the pressure cause and
the test still passes green, guarding the pressure path instead of the
512-cap. Concrete scenario: a `prlimit 650` full-suite run → these two pass
while proving nothing about the budget they name.

Not the strand hazard (no headroom check inside the closures, so no
in-closure panic to strand; the test thread holds nothing in those tests,
so the in-closure `hold_flood_lock()` cannot deadlock) — this entry is
purely about cause-pinning.

## Design decision required (not just mechanics)

`assert_shm_protocol_error` asserts on code rather than message
*deliberately* ("stays valid whichever side answered"). Reusing
`assert_raw_protocol_error_with_message` here overturns that stance for
these two tests, so the fix must say which stance wins and why — the
siblings' answer (message discrimination is the only signal that
distinguishes the causes on a shared code+object) presumably carries, but
make it explicit rather than drifting the helper's contract by example.

## Out of scope

Production behavior (both refusals are correct — test-only), CI/ulimit
changes, the `msg_broken_pipe` accept-hang (separate ticket, different
mechanism).
