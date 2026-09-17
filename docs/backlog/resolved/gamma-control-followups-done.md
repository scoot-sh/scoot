---
title: "Gamma-control follow-ups from PR #32's review (dead `stored` field, doc overprecision, untested superseded-destroy gate) — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Gamma-control follow-ups from PR #32's review (dead `stored` field, doc overprecision, untested superseded-destroy gate) — DONE

## Resolution (PR #78)

All three bundled as filed, minimal and proportional:

1. **Dead `stored` field — REMOVED, not justified.** The ticket's bar for
   keeping it was naming a REAL planned consumer of a "report the current
   ramp" surface. None exists: `flexwm-ipc` has zero gamma/ramp references,
   no backlog item asks for current-ramp reporting, and no production code
   reads `stored` (only the write in `set_gamma` and the clear in
   `restore_default`). The Noctalia probe's night-light toggle produces no
   gamma wire traffic at all, so not even a shell is asking. Removed the
   field, its write/clear, and the two test asserts on it, plus the
   misleading write-only "record of what is on screen stays true" comment.
   Safe by construction: `restore_default` recomputes the linear ramp fresh
   via `linear_ramp()` rather than reading `stored`, and no tty hotplug/VT
   path re-applies from it (`set_gamma_ramp` has exactly two call sites,
   both in-module). `set_gamma` is restructured around the removal (early
   return outside `--tty` after the length check; ramp decode only where a
   LUT exists), with accept semantics unchanged on every backend.
2. **Doc overprecision — fixed.** `read_positioned` appends whole 4096-byte
   chunks until past `limit`, so transient allocation is up to `limit +
   4096`, not `limit + 1`. Fixed in the module doc, `read_bounded`'s doc,
   `read_positioned`'s doc, and the `gamma_oversized_fd_refused` test
   comment. `read_sequential`'s `limit + 1` stands (stated in place: `take`
   stops mid-chunk, so it is exact). Comment-only; the bounded/refused
   security property holds as before.
3. **Superseded-destroy gate — tested fail-first.** New
   `gamma_destroying_superseded_control_restores_nothing`: transfer to a
   second control, destroy the OLD one, create a third — if the second was
   still live the transfer fails it, if a restore retired it nothing fails.
   Neutered-gate run (restore-on-any-destroy) fails with `Err("the
   compositor never sent failed on the still-live second control")`;
   reverted, it passes. First draft asserted `current.is_some()` after
   settle and failed even unfixed — the disconnect at client-thread exit
   legitimately retires the live control (the tested
   disconnect-restores-default behavior) — so the test is purely
   client-observable by design, plus a final `current.is_none()` pinning the
   live third control's destroy restoring the default.

No benchmark (per-request/per-destroy paths only, nothing hot-path).
`scripts/smoke-test.sh` skipped by judgment (gamma is headless-invisible;
no IPC/window/render/input path touched). README: one word dropped
("accepted and stored" → "accepted") to avoid lying post-removal — no new
user-facing surface, so no surface docs.

Original entry, left as written:

# Gamma-control follow-ups from PR #32's review (dead `stored` field, doc overprecision, untested superseded-destroy gate).

Three non-blocking findings from the independent review of PR #32
(`gamma-data-control-globals`, merged 2026-09-14), recorded here so
they don't get lost. None is near the merge bar on its own; bundle
them with the next touch of `compositor/gamma_control.rs`:

1. **Dead state** — `compositor/gamma_control.rs`'s `stored` field is
   written in `set_gamma` and cleared in `restore_default` but never
   read by production code. Either justify it as a future IPC surface
   (e.g. reporting the current ramp) or remove it; harmless (~4.6 KB
   retained per ramp until destroy).
2. **Doc overprecision** — the `read_positioned` bound comment
   (`gamma_control.rs`, plus the `gamma_oversized_fd_refused` test
   comment): reads happen in 4096-byte chunks, so transient allocation
   is up to `limit + 4096`, not `limit + 1`. The security property
   (bounded, oversize refused) holds; only the comment's arithmetic is
   off.
3. **Untested path** — the superseded-control-destroy identity gate in
   `destroyed` (the `!is_current → no restore` arm) has no dedicated
   test; the transfer test never destroys the old control. Six obvious
   lines, but exactly the 5b-class gate this project tests explicitly
   (a field correct at its own site but conflated elsewhere).
