---
title: "Dumb-tier buffer age advances while Smithay freezes damage history on empty damage (cursor-driven full-repaint alternation)"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Damage-history desync on the dumb tier

Found 2026-09-26 while investigating `multi-output-remainder.md` item 3
(per-output render scheduling): that item's "~1pp no-damage pass" premise
dissolved on measurement — about half the "no-damage" frames are full
repaints. The scheduling investigation reported back don't-build with
numbers; this ticket is the real bug it found. No code changed yet.

## Mechanism (verified against pinned Smithay `5b57532`)

- Smithay `damage/mod.rs`: `if self.damage.is_empty() { "nothing damaged,
  exiting early"; return … }` runs BEFORE `old_damage.push_front` — on an
  empty-damage pass the history freezes. Age 0 takes the "no old damage
  available, re-render everything" arm (full `output_geo` damage).
- scoot `render.rs` calls `tty.advance_generation(id)` unconditionally on
  every `Ok` render, and `tty/buffers.rs`' doc claims Smithay's history
  "advances unconditionally on every `render_output` call … the history
  push happens before the check that might skip drawing". That claim is
  **false** for the empty-damage case (see above) — the doc needs
  correcting with the fix.
- Steady state: a cursor-only frame reports damage-None and advances the
  age with no history growth; the next frame finds `age > old_damage.len()`
  and repaints everything. Census on the dev VM (release, vkms 1024x768,
  dumb/pixman): 20 same-position moves → 11 incremental + 9 full-repaints
  (10 damage-None); 20 wandering moves → 20 incremental, 0 full-repaints.

## Fix direction (design open, scoped)

Gate `advance_generation` on `render_result.damage.is_some()` in
`draw_frame_with` — exact correspondence (None ⟺ early return ⟺ zero
history mutation), dumb-tier-scoped; age-0 retry still full-damages then
advances. Needs: the `buffers.rs` doc correction, a contract test plus a
red regression test (render, render-same → None, render-same → still None),
and a full cycle including vblank/lock bug-bash (fewer spurious flips
changes present cadence). Expected win: ~half the full repaints gone from
every cursor-driven stretch — bigger than item 3's 1pp at a fraction of a
skip's risk. Adjacent, price separately: `world.arrange()` runs per output
per frame.

## Why item 3 stays don't-build

Headless/nested/GLES-offscreen pin age 0 (always full-damage, nothing to
skip); scanout owns its damage; on the dumb tier a skip would drop
per-frame callbacks, lock-blank arming, and layer cleanup, and deciding
the skip needs shadow dirty tracking across ~25 call sites or a
per-frame probe walk. Full analysis in the 2026-09-26 scheduling
investigation (no PR — reported with numbers, `sched-*.log/.png` on the
dev VM's `/tmp`).
