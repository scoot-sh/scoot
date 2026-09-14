---
title: "`wlr-layer-shell-unstable-v1` protocol support \u2014 DONE as item 14"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `wlr-layer-shell-unstable-v1` protocol support — DONE as item 14

~~`wlr-layer-shell-unstable-v1` protocol support~~ — DONE as item 14,
except for layer popups (the entry two below). The original entry's guess about the shape turned out half
right: the `Elements` enum did need changing, but into *one* `Surface`
variant rather than a fourth, and the exclusive-zone plumbing into
`flexwm-core` was one new field plus one new event, not a restructure.
