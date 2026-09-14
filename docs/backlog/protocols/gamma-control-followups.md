---
title: "Gamma-control follow-ups from PR #32's review (dead `stored` field, doc overprecision, untested superseded-destroy gate)."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

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
