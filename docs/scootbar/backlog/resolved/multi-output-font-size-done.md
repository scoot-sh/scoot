---
title: "Per-output font size"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: null
resolved: "2026-10-08"
---

# Per-output font size

## Resolution (2026-10-08)

Landed as designed, in #520: `[output."NAME"]` takes `font-size` (1 to
256, as `[bar] font-size`; absent is the shared value), and each output
draws at its own em. `Override` carries the size, `Resolved` and
`Placement` thread it, `Objects` keeps it per output (set at settle and on
reload, so hotplug and scale changes keep it), and the scene measures,
`paint`, the click hit-test and popup/tooltip layout read the output's
own; everything else in `Style` stays shared. A `font-size`-only reload
makes no surface again (the geometry did not change). Verified with the
scootbar standard set on the Asahi M2 (35-combo clippy matrix, nextest
1398 passed, `cargo test` green with all three REQUIREs, deny, site build),
the revert-run-restore unit proofs, a two-output headless-scoot test
(different ems from one config; a reload moving one output's size moves
only its modules, same surfaces and fds), and the ratchet (release file
+0 B, `.text` +1,248 B unwaived; idle RSS/wakeups level). Reference:
`site/src/content/docs/scootbar/` (`cli.md`, `configure.md` Outputs).

**Deferred 2026-10-01, by the maintainer: not scheduled until someone asks for it.** It
leaves M3 (the milestone is done without it); nothing here is blocked, and the
design below stands for whoever picks it up.

Filed 2026-09-30, the remainder of [multi-output](multi-output-done.md). Serves
**daily-drive**: a 4K panel next to a 1080p one wants a different em, not
only the same em at its own device pixels.

## The gap

`[output."NAME"]` (see [cli.md](../cli.md#outputs)) overrides the bar's
geometry and module lists, but the em (`[bar] font-size`), like every part
of `Style`, is shared: `Content.style` is one value that `Scene::update` and
`render::paint` read for every output. Each bar already draws at its own
output's real device pixels, so the same 14 logical pixels are sharp at any
scale; a different size per output is not expressible.

## What to do

Add `font-size` to the output table, validated as `[bar] font-size` is (1 to
256), and carry the size per output instead of in the shared `Style`: the
scene measures at it, `paint` and the click hit-test read the output's own.
The glyph cache is keyed by size already, so a second size costs its glyphs
only. Pin: a reload that changes an output's size re-measures and redraws
only that output, and `padding` and `spacing` stay shared unless asked.

## Not in this ticket

Per-output colors, padding, spacing, radius and opacity.
