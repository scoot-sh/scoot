---
title: "corner_radius surface clip: runtime toggle, opaque-region coverage, X windows"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-02"
---

# corner_radius surface clip: runtime toggle, opaque-region coverage, X windows

Filed 2026-10-30 from the review of PR #825 (clip a window's surface tree to
its geometry under non-zero `corner_radius`). Serves **daily-drive**.

## The gap

1. **A runtime `corner_radius` 0 to non-zero toggle may leave stale margin
   pixels.** Unverified. With `corner_radius = 0` the window is unwrapped and a
   client-drawn frame outside its geometry (Chrome's 1px border, GTK shadows)
   paints into the gap. After a reload to a non-zero radius the `Rounded`
   wrapper clips that margin, but `Rounded` forwards the inner element's
   `current_commit` (`crates/scoot/src/compositor/rounded.rs`), so a change of
   radius alone does not bump the commit counter. `finish_reload`
   (`crates/scoot/src/compositor/reload.rs`) only calls `request_render()` for
   `CORNER_RADIUS`, which sets `needs_render` and `scene_dirty`; the latter
   feeds frame-serial accounting, not a forced full repaint. On a
   damage-tracked backend (nested, tty) the old margin pixels could stay until
   the client next commits. Trace whether the damage tracker repaints the
   window on a wrapper change, and if not, reproduce on `--nested` (reload with
   the radius changed, screenshot the gap) before fixing.
2. **The opaque-region cut is covered only at unit level.** Headless redraws
   the whole frame each time, so no end-to-end test would notice the opaque
   region reaching past the clip (which would let the damage tracker skip what
   is under pixels the draw leaves untouched). Cover it on a damage-tracked
   path.
3. **X windows are not clipped, and the docs do not say so.** The wrapper
   covers a toplevel's Wayland surface tree; XWayland windows keep their
   unclipped margin. State that on the corner_radius docs page, or extend the
   clip.

## What to do

Trace (1) first: it is the only item that could be a visible bug. (2) and (3)
are a test and a docs sentence.

## Not in this ticket

Clipping popups (deliberately never wrapped).

## Resolution

Landed in PR #868 (`fix(scoot): repaint the whole output on a corner_radius reload`).

1. **Real bug, fixed on both tiers.** Traced against the pinned Smithay fork (`b65cbc5`): an unchanged element contributes only `damage_since` at its stored commit, and `Rounded` forwards the inner commit, so a radius change alone damages nothing; only buffer age > 0 reads damage history, and only `--tty` passes a real age. `finish_reload` now resets every backend's `OutputDamageTracker` (`Backend::reset_damage`) on a `CORNER_RADIUS` change -- one full repaint on the next frame, cold path only -- and arms one composite-only frame plus a swapchain-slot invalidation (`arm_force_composite` + `invalidate_scanout`, the same pattern as `force_scanout_composite` for stale captures) on every output's scanout presenter. The backend reset cannot reach the scanout tier (`DrmCompositor` owns its own tracker; the reset's doc says so), and an empty scanout frame keeps the old pixels on screen, so without the presenter half a GPU `--tty` session would keep the stale margin/corner pixels exactly like the dumb tier did. The presenter lookup is a no-op where there is no scanout presenter (headless/nested, dumb-tier heads, builds without `gpu-scanout`); overlay/cursor planes need no separate reset (the cursor rides its own element with its own commit). Live `--nested` repro (cage host, foot client, 0 to 12 + reload) repaints correctly with no stale pixels, as the age-0 full-redraw analysis predicts; the age-1 regression tests (`a_corner_radius_reload_forces_full_damage_on_a_tracked_backend` for the backend reset, `a_corner_radius_reload_visits_every_outputs_scanout_presenter` for the per-output scanout loop) are the damage-tracked-path proof and fail without their half. Sibling reload keys do not share the bug, so the scope stays narrow: `BACKGROUND` forces full damage via Smithay's own clear-color change arm, ring width/color ride `SolidColorBuffer`/`MemoryRenderBuffer` commits (and width moves geometry), gaps go through `apply()` re-arrange -- nobody should 'fix' them.
2. **Opaque cut covered on a damage-tracked path.** `what_is_below_the_cut_repaints_when_it_changes` renders through a real pixman damage tracker at age 1; neutering the full cut or just the corner-square subtraction fails the first frame's corner assert, neutering just the geometry-bounds clip fails the first frame's margin assert.
3. **X windows documented, not extended.** The `corner_radius` page states XWayland windows are rounded at the corners but keep their margin: no `xdg_toplevel` geometry to clip to, X geometry semantics unverified.
