---
item: "5"
title: "Cursor rendering for --tty"
status: "done"
area: "rendering"
pr: 8
commit: "546534b"
---

# Cursor rendering for --tty

~~Cursor rendering for `--tty`~~ — DONE, merged to `main` at `546534b`,
PR #8. `crates/flexwm/src/compositor/cursor.rs`: a persistent,
procedurally-generated arrow bitmap (no copied cursor-theme asset —
deliberately license-clean per `CLAUDE.md`) rendered front-most, gated to
`--tty` only. Damage-scoped presentation: `tty/buffers.rs` tracks each
dumb buffer's own age (the two slots don't age in lockstep) so
`tty::present()`'s memcpy scopes to the damaged bounding box instead of
the full frame on every mouse-motion-triggered render. Independent review
found two real issues, both fixed before commit: (a) an off-by-one in the
age calculation (`generation - last_written + 1`, not `generation -
last_written` — the render being peeked-for hasn't happened yet) that
would have left stale pixels on the scanout buffer under slot alternation,
invisible to every existing check since they all read the pixman
intermediate, never the actual DRM buffer this lived in; (b)
`write_region`'s pitch/offset copy math had zero test coverage, closed by
extracting pure `age`/`copy_region_rows` helpers with regression tests.
Both fixes are unit/source-verified against the pinned Smithay damage
tracker, not empirically screenshot-verified — screenshots structurally
can't observe the scanout buffer. Benchmarked via jiffies-delta: idle
unchanged at 0, ~7 jiffies/150 small local moves vs ~42/150 near-full-frame
corner jumps (the latter ≈ pre-fix cost) — confirms the scoping fix is
real. `CursorImageStatus::Named`/`::Surface` both drew the same fallback
shape; `::Surface` was fixed in item 8. `::Named` still draws the fallback
and always will (there is no client buffer behind a name) — item 13 made
that shape's size and color configurable; a *per-name* shape is still
Backlog (b) and needs a licensed asset source first.
