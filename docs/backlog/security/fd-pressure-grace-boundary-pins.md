---
title: "Pin the fd-pressure grace conjunction's boundaries (>, &&, 128/129, 64/65)"
status: "open"
area: "security"
priority: "low"
blocked: null
---

# Pin the fd-pressure grace conjunction's boundaries

Filed from PR #123 review (2026-09-18). Seven tests pin `Table::pressured`
exhaustively, but `pressure_refusal(live, grace)` in `dispatch.rs`
(`live > grace && table().is_some_and(pressured)`) has zero tests: no pin
on `>` vs `>=`, no pin on `&&` vs `||`. An operator flip there kills
under-grace clients during pressure — the exact catastrophe the grace
exists to prevent — with no suite failure. The shipped operator is
correct as written (review-traced at all four call sites); this is about
pinning it, not fixing it.

Suggested shape: split the pure conjunction out for testability and pin
the 128/129-buffer and 64/65-pool boundaries. Note the related exactness
detail: `live > grace` permits grace+1 units (129 buffers / 65 pools), so
any "two at grace" arithmetic in the record is really +4 fds — negligible,
but state it exactly when writing the pins.
