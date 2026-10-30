---
title: "corner_radius surface clip: runtime toggle, opaque-region coverage, X windows"
status: "open"
area: "core"
priority: "low"
blocked: null
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
