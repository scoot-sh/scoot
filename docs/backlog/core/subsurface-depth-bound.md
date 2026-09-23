---
title: "Deeply nested subsurfaces overflow the compositor's stack — any client can crash every session"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Bound subsurface nesting depth (client-triggerable crash)

Filed 2026-09-23 while implementing the
[popup depth bound](../resolved/popup-depth-bound-done.md), from reading
the pinned source; **measured** by the review of PR #226 (at `10e6b53`,
pre-existing -- nothing in that PR touches subsurfaces). Serves both
priorities for the same reason the popup bound did: a compositor crash
takes every client's unsaved state with it.

## Measured

A client maps a window, then builds `N` desync `wl_subsurface`s in one
batch, each the child of the one before, each with a 4x4 buffer, and
commits the window; the harness then draws a frame and checks that a
second client is still served (a `SubChain { len }` op in the
`popup_parent/tests` harness shape):

| build | stack | result |
|---|---|---|
| debug | 2 MB (test thread) | 1000 levels fine (7 ms frame); 3000 overflows |
| release | 2 MB | 3000 fine (2.3 ms frame); 10000 overflows |
| release | 8 MB (a real session's main thread) | 30000 overflows |

## Why

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

A fail-first harness test at a depth that crashes the debug build, then a
cap checked in scoot's own `new_subsurface` hook (or
wherever the pinned rev offers one) with a protocol error, the way the
popup cap is. `wl_subsurface`'s parent is fixed at creation, and a
surface's subsurface role cannot be re-parented, but check the pinned
source for the equivalent of the popup re-parenting bypasses before
relying on that.
