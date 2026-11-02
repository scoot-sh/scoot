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

1. **Real bug, fixed.** Traced against the pinned Smithay fork (`b65cbc5`): an unchanged element contributes only `damage_since` at its stored commit, and `Rounded` forwards the inner commit, so a radius change alone damages nothing; only buffer age > 0 reads damage history, and only `--tty` passes a real age. `finish_reload` now resets every backend's `OutputDamageTracker` (`Backend::reset_damage`) on a `CORNER_RADIUS` change -- one full repaint on the next frame, cold path only. Live `--nested` repro (cage host, foot client, 0 to 12 + reload) repaints correctly with no stale pixels, as the age-0 full-redraw analysis predicts; the age-1 regression test (`a_corner_radius_reload_forces_full_damage_on_a_tracked_backend`) is the damage-tracked-path proof and fails without the reset.
2. **Opaque cut covered on a damage-tracked path.** `what_is_below_the_cut_repaints_when_it_changes` renders through a real pixman damage tracker at age 1; the corner and margin probes fail when the cut is neutered.
3. **X windows documented, not extended.** The `corner_radius` page states XWayland windows are rounded at the corners but keep their margin: no `xdg_toplevel` geometry to clip to, X geometry semantics unverified.
