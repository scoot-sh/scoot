---
title: "Fractional-scale ring-hole drift in the painted-ring origin refresh — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Fractional-scale ring-hole drift in the painted-ring origin refresh — DONE

RESOLVED 2026-09-21. Filed 2026-09-20 from `scoot-reviewer`'s re-review
pass on PR #184, which landed position-refresh for the painted ring
strips and recommended this as the follow-up rather than scope for that
PR. Fixed exactly as the ticket prescribed: compare `plan.inner` /
`plan.outer` too in the reuse check.

## Mechanism (as filed; reproduced in-harness fail-first below)

`refresh_strip_origins` (`crates/scoot/src/compositor/decorations.rs`)
compared only strip canvases to decide buffer reuse, but paint content
(`inner`/`outer` in-canvas offsets) also depends on absolute position
through rounding phases. Concrete instance from the ticket (scale 1.25,
thickness 2, 100px window, x=1→2): canvases identical → no repaint, but
`inner.x` moves 2→3, so the reused buffer lands the ring hole 1px off
the window clip until a key change (color/size) repaints.

One subtlety the fix documents at the comparison site: `outer` is
`(0, 0, canvas)` and is compared explicitly even though the strip
canvases are already compared, because the full-canvas height is
invisible in the strip canvases — shifting it by a row shifts the bottom
strip's origin by the same exact amount, leaving both strip canvases
unchanged while the bottom strip's paint shifts. Conversely, whether a
scale/thickness pair *can* drift is arithmetic, not a bug: when
`thickness * scale` is an integer there is no rounding phase for the
offsets to move through (e.g. thickness 2 at 1.5 never drifts on 1px
steps), so every move there stays a clean hit or a canvas-driven
repaint.

## Resolution

`PaintedRing` stores the full-canvas paint inputs it was painted from
(`inner_at` / `outer_at`, set alongside `top_at` / `bottom_at` in
`build_strips`, cleared on every failure path including the import-time
poisoning in `push_painted`), and `refresh_strip_origins` returns
`false` (repaint) when either disagrees with the recomputed plan.
Still allocation-free: `plan_strips()` already computes the full plan,
so this is one more comparison on values already in hand, no new
geometry, no renderer touch on the hit path. Production behavior change
is confined to repainting (rarely) more often on fractional scales —
never less correctly. Integer-scale behavior is byte-identical: neither
offset can move without the key there, so the added comparison never
fires (pinned by test, not reasoned).

No benchmark re-measurement owed: the hit path gains two
`Option<Rectangle<i32, Physical>>` comparisons (a tag check plus four
`i32` compares each) on the radius>0 path only; the square path is
untouched and the steady-state shape (reuse with two origin stores, no
allocation, no renderer touch) is unchanged.

## Proof

Regression tests in `crates/scoot/src/compositor/decorations.rs`
(`mod tests`, Linux-only like the rest of the compositor):

- `fractional_scale_move_with_matching_canvases_still_repaints` — the
  ticket's x=1→2 instance at scale 1.25, with the mechanism pinned as
  preconditions (canvases equal, `inner` differs). **Failed pre-fix**
  (`canvas-matching move at scale 1.25 must repaint`), passes post-fix.
- `fractional_sweep_repaints_exactly_on_plan_disagreement` — the
  reviewer's brute-force shape in-harness (real `plan_strips` + real
  `refresh_strip_origins`, not a rounding replica): scales
  1.25/1.5/1.75/1.33 × thicknesses 1..=4 × 1px moves in x and y, asserting
  the refresh repaints exactly when the plans disagree on a canvas or an
  offset (no missed drift, no spurious repaint), and asserting each scale
  actually produced canvas-matching drift pairs (a scale with zero would
  pin nothing). **Failed pre-fix**, passes post-fix.
- `integer_scale_moves_never_repaint` — scales 1.0/2.0/3.0 × 40 moves:
  always a hit. Passed pre- and post-fix (zero behavior change).
- `repaint_after_drift_settles_into_a_hit` — rebuild after a drift miss,
  then refresh: hit (no every-frame oscillation). Failed pre-fix on its
  first assertion, passes post-fix.
- `refresh_on_a_poisoned_entry_reports_a_miss` — the #184 poison contract
  (`top_at.is_some()` guard): a never-built entry reports a miss without
  populating anything. Passed pre- and post-fix.

Standard gate (all against the fixed tree on the dev VM over the 9p
mount, plus fmt Mac-side):

- `cargo nextest run --workspace` — 1303 passed, 6 skipped, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check --all` — clean (after one `cargo fmt -p scoot`).
- `scripts/smoke-test.sh` (SMOKE_PREFIX=/tmp/smoke-drift,
  CARGO_TARGET_DIR=/var/cargo-target) — 19 `ok` lines, zero failures.

Live fractional-scale proof (headless honors `[output] scale`, so this
was reachable): a `--headless` session with `[output] scale = 1.25` +
`[appearance] corner_radius = 8`, two `foot` windows, `move-column`
both directions, three `screenshot` captures (1600x1000 physical =
1280x800 logical @1.25 ✓), zero warn/error/panic in the compositor log,
and ring pixels verified with `magick` in the captures (inactive-ring
`(89,89,97)` exactly at the unfocused window's ring row, background
`(20,20,25)` elsewhere). The drift itself is pinned in-harness rather
than live: a 1px ring-hole offset on one rounding-phase move is not
something a layout-driven session can aim at pixel-exactly, while the
harness drives the production `plan_strips`/`refresh_strip_origins`
directly at the exact instance — stated, not papered over.

## Out of scope (per the ticket)

No other render-path change, no integer-scale behavior change, no CI
changes, no README change (no user-facing surface: a fractional-scale
1px correctness fix with no new option, flag, binding, or IPC shape).
