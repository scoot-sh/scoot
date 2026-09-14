---
title: "`--width`/`--height` are unbounded `i32`s (LOW, operator-supplied)."
status: "open"
area: "core"
priority: "low"
blocked: null
---

# `--width`/`--height` are unbounded `i32`s (LOW, operator-supplied).

`--width`/`--height` are unbounded `i32`s (LOW, operator-supplied).
Found while bug-bashing item 12 and deliberately left out of it. Item 12(b)
bounds a *client's* `min_size` to the output's usable area, and 12(c) bounds
the gap, but the output's own size still comes straight from
`cli.rs`'s `number("--width", ..)` with no range check — so
`--width 2000000000` can overflow the same `x + width` in `arrange.rs`'s
on-screen test that 12(b) closes for minimums, plus `Rect::right()`/
`bottom()` wherever those are read. Lower priority than the four in item 12
because it needs the operator to pass an absurd flag to their own
compositor rather than a client or a config file to declare one, but it is
the same family of fix (clamp at the read site, with a documented bound)
and would close the last unbounded input to the layout's arithmetic. A
realistic bound is whatever DRM itself can report for a mode, with room to
spare. Two more facts `flexwm-reviewer` found while reviewing item 12,
worth fixing alongside this rather than separately: an absurd `--width`
doesn't just overflow directly — since 12(b)'s `min_size` limit is
*derived from* the output's usable area, a huge enough output makes that
clamp effectively vacuous (the limit becomes ~2×10⁹, which bounds nothing
real); and `Rect::inset`'s `self.x + by`/`self.y + by` are unguarded even
though its `w`/`h` arms already floor at 0 — the same function, only half
hardened.
