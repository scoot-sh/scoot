---
title: "Deeply nested subsurfaces overflow the compositor's stack — any client can crash every session — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Bound subsurface nesting depth (client-triggerable crash) — RESOLVED

RESOLVED 2026-09-23 (PR #227). The rule, and why, is in
`crates/scoot/src/compositor/subsurface_depth.rs`; the tests in
`subsurface_depth/tests/`, driving `popup_parent`'s client (now shared,
with subsurface ops in `popup_parent/tests/subsurfaces.rs`).

- **Cap: 64 levels of subsurface below a tree's root** (the surface that
  is not itself a subsurface: a window, popup, layer surface, cursor, or
  a plain surface with no role). Refused as `wl_subcompositor.bad_parent`,
  posted on the `wl_subcompositor`.
- **Checked before Smithay sees the request**, in `dispatch.rs`'s blanket
  `request` (a new guard, `reject_too_deep_subsurface`), not in
  `CompositorHandler::new_subsurface` as this ticket suggested: Smithay's
  `get_subsurface` handler links the surfaces, after running its own
  recursive `is_ancestor` up the new parent's chain, *before* it calls
  `new_subsurface`, and does not pass it the `wl_subcompositor`. Refused in
  the guard, the link is never made, and `is_ancestor` only ever walks a
  chain the rule already admitted -- which is what bounds it.
- **The ticket's "a subsurface role cannot be re-parented" was not enough
  to rely on.** A role outlives its `wl_subsurface`: after
  `wl_subsurface.destroy` (legal) or after the parent `wl_surface` is
  destroyed (Smithay then accepts a second `get_subsurface` although the
  first `wl_subsurface` is alive -- filed, see below), the surface can be
  made a subsurface again, elsewhere, with everything below it still
  attached; and a role-less surface can be given subsurfaces before it
  becomes one. So a link does not always add a leaf, and a cap on the new
  parent's depth alone is bypassed by assembling a tree bottom-up from
  short pieces -- measured below: that alone overflowed the stack. The
  check is `depth(parent) + 1 + height(attached subtree) <= 64`.
- **The height is a bound, not a walk:** each surface carries an upper
  bound on its subtree's height (`SubtreeHeight`, an atomic in its data
  map), raised along the new parent's chain in `new_subsurface` and never
  lowered. So the check is one walk up at most 64 links and a read,
  whatever the width of the client's trees, and nothing is added to the
  per-frame path. Not lowering it is a choice, not a missing hook
  (`dispatch.rs`'s blanket `destroyed` sees every `wl_subsurface` and
  `wl_surface` go): lowering means recomputing each ancestor's height from
  all of its children, work per level in proportion to the tree's width.
  Its one error is conservative and pinned by a test: a surface that once
  had a subtree `h` deep, re-attached `d` levels down with `d + 1 + h > 64`,
  is judged by the subtree after it is gone. The deepest tree measured
  from a real client is 2 levels (mpv), so that needs trees about thirty
  times deeper than any seen.
- **Unlike the popup tree there is no second structure to drift:** a
  surface's `parent` and its parent's `children` are written together at
  all three sites (`set_parent`, `unset_parent`, `cleanup`), and only
  `set_parent` adds a link.
- **Popups do not multiply it:** `PopupManager::popups_for_surface`
  collects a window's popups before any is walked, so a popup's surface
  tree is walked on its own, never nested inside the popup walk. A frame
  with a 64-deep popup chain and a 64-deep subsurface chain below its
  deepest popup is drawn by the test suite (debug, 2 MB stack) and costs
  what it did before (below).
- **A cycle is still Smithay's:** a parent that is the surface or one of
  its descendants is left to Smithay's own `bad_surface`, which the guard
  detects on its walk up.

**What is guaranteed:** no surface is ever more than 64 subsurface levels
below its tree's root, so every Smithay walk over a surface tree
(`map_tree` behind `with_surface_tree_*`, `commit_sync_surface_tree`,
`is_effectively_sync`, `is_ancestor`) and scoot's own
`resend_scale_tree` recurses at most 65 levels.

**Not covered, and filed:** [Smithay accepts a second `wl_subsurface`
for an orphaned subsurface](../core/subsurface-second-wl-subsurface.md) --
low; not a depth path (the link goes through the guard like any other).
[Many desynchronized subsurfaces in one window stall roughly
quadratically](../core/subsurface-count-quadratic.md) -- medium,
measured: many subsurfaces *side by side*, not deep, which this bound does
not touch.

## Evidence

Cache key: everything below was captured against the working tree
committed unchanged as `3bfdeda`; the commits after it change only
comments and docs (`git diff 3bfdeda HEAD -- crates/ | grep '^[+-][^+-]'
| grep -v '^[+-]//!'` prints nothing). Raw artifacts: dev VM
`/tmp/subsurf-evidence/`.

Base: `7017883` exported on the dev VM with `git archive 7017883 | tar -x
-C /tmp/subsurf-base` (the VM cannot write the 9p-mounted repo's `.git`,
so `git worktree add` there fails -- `cargo fmt` from the VM already hit
`Permission denied` on the mount), the new test files copied in with a
stub `subsurface_depth.rs` that only mounts them, all sources `touch`ed,
built in `CARGO_TARGET_DIR=/tmp/subsurf-base-target`.

Base, debug, `cargo nextest run -p scoot --no-fail-fast -E
'test(/subsurface_depth/)'` (2 MB test-thread stack): 5 passed (the
at-cap and legal re-attach tests, which must pass both ways), 10 failed.
Three by stack overflow (`SIGABRT`, "has overflowed its stack"): a
3000-deep desync chain (at the frame drawn after it), the bottom-up
assembly (50 pieces of 61), and a 20000-deep chain under a role-less
surface nothing draws. That last one was probed with an `eprintln!`
around the delegated request: the last lines are `enter get_subsurface
8588` with no `left`, so it overflowed inside Smithay's `get_subsurface`
handler, whose only recursion is `is_ancestor`. The 3000-deep
*synchronized* chain did not crash: its dispatch took over 30 s (the
harness timed out at 37 s). That stall scales with depth, and so is now
bounded too: 3000 synchronized subsurfaces *side by side* under one window
(a throwaway probe on this branch, not committed) took 197 ms in debug.
The rest failed as "the client survived".

Mutation (the branch's code with `subtree_height` returning 0, i.e. a
cap on the parent's depth alone), bypass tests: the bottom-up assembly
overflowed the stack (`SIGABRT`) and all four re-attach refusals were
admitted.

Release, `SCOOT_SUBSURFACE_DEPTH=N cargo test --release ...
subsurface_chain_cost -- --ignored --nocapture` (the test binary run
directly):

| depth, stack | base `7017883` | this branch |
|---|---|---|
| 3000, 2 MB | admitted after 552 ms; next frame 1.30 ms | refused after 12.9 ms; next frame 9 µs |
| 10000, 2 MB | stack overflow | refused after 66.7 ms; next frame 19 µs |
| 30000, 8 MB | 108 s dispatch stall, then stack overflow | refused after 548 ms; next frame 23 µs |

(The refused times are the client writing the rest of its batch into a
closed socket.) At the cap, two alternating rounds each, best of 5x500
frames, 200x200:

| frame | base | branch |
|---|---|---|
| window alone | 11.6 / 11.5 µs | 11.1 / 11.1 µs |
| window + 64 nested subsurfaces | 40.9 / 40.5 µs | 41.2 / 41.5 µs |
| window + 64 nested popups | 214.0 / 214.2 µs | 206.2 / 205.9 µs |
| ... + 64 nested subsurfaces below the deepest popup | 252.7 / 251.7 µs | 248.5 / 247.6 µs |
| create 64 nested subsurfaces, one batch (round-trip floor) | 20.2 (18.9) / 19.7 (18.7) ms | 19.4 (18.3) / 19.8 (18.6) ms |

Real clients, live (branch debug build, `--headless`, `WAYLAND_DEBUG=1`,
6 s each, a screenshot each): weston-subsurfaces 16.0.0 (two sibling
subsurfaces), foot with `csd.preferred=client` (one), gtk4-demo's video
player, GTK 4.22 (one), and mpv 0.41 `--vo=gpu --gpu-context=wayland`
(two, **built bottom-up**: `get_subsurface(#7, #6)` before `#6` has a
role, then `get_subsurface(#6, #5)` -- the attach-a-subtree path the
height bound exists for) all kept running with no protocol error and no
refusal in the compositor log; weston's and mpv's frames drawn intact.
gtk3-widget-factory made no subsurfaces. gtk4-widget-factory aborted on
its own icon loader (`Gdk-ERROR ... not a valid image`) and mpv
`--vo=dmabuf-wayland` failed its own `hwupload`, both before drawing and
unrelated to scoot.

---

The original report follows.


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
