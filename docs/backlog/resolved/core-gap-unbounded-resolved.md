---
title: "`flexwm-core`'s `gap` config value has no upper bound (LOW) \u2014 DONE as item 12(c)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flexwm-core`'s `gap` config value has no upper bound (LOW) — DONE as item 12(c)

~~`flexwm-core`'s `gap` config value has no upper bound (LOW)~~ — DONE as
item 12(c), `Config::MAX_GAP` = 10,000 with `clamp_gap` shared between the
core's `validated()` and the compositor's focus-ring sizing. Original
diagnosis, left as written: clamped
only at the bottom (`.max(0)`); a very large configured gap can overflow
plain `i32` arithmetic in `layout.rs`/`arrange.rs`. Config-only, same fix
shape as the `min_size` finding above.
