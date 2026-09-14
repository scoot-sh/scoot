---
title: "Lock surfaces are per-output and flexwm has one output"
status: "open"
area: "protocols"
priority: "low"
blocked: "needs multi-output"
---

# Lock surfaces are per-output and flexwm has one output

Lock surfaces are per-output and flexwm has one output (item 18).
`new_surface` honours the `wl_output` the client named but falls back to
the single output; `configure_all` resizes them all together. One more
site for the multi-output list `headless.rs`'s `OUTPUT_ID` doc keeps.
