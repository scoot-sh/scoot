---
title: "Deeply nested subsurfaces may overflow the compositor's stack — the popup depth bound's sibling, unmeasured"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Bound subsurface nesting depth (suspected client-triggerable crash)

Filed 2026-09-23 while implementing the
[popup depth bound](../resolved/popup-depth-bound-done.md). **Found by
reading the pinned source, not measured** -- the first step is a
fail-first test that shows whether it is real (see `CLAUDE.md`'s
"establish whether the harm is real" rule).

## What looks wrong

A `wl_subsurface` can be the parent of another, so a client can nest them
as deep as it likes, and Smithay's walks over a surface tree recurse once
per level at the pinned rev (`src/wayland/compositor/tree.rs`):

- `PrivateSurfaceData::map` -- behind `map_tree`, and so behind
  `with_surface_tree_downward`/`_upward`, which the render path runs over
  every window's surface tree each frame -- recurses per child level and
  holds each level's surface lock while it does;
- `PrivateSurfaceData::is_ancestor`, run by `set_parent` on every
  `wl_subcompositor.get_subsurface`, recurses per ancestor.

That is the same shape as the popup tree's recursion, which overflowed a
2 MB stack at about 2000 levels in debug (10000 in release). The popup
bound (`popup_parent.rs`) does not cover it: a subsurface is not a popup.

## What a fix needs

A measured depth at which it breaks (debug and release, frame and
creation), then a cap checked in scoot's own `new_subsurface` hook (or
wherever the pinned rev offers one) with a protocol error, the way the
popup cap is. `wl_subsurface`'s parent is fixed at creation, and a
surface's subsurface role cannot be re-parented, but check the pinned
source for the equivalent of the popup re-parenting bypasses before
relying on that.
