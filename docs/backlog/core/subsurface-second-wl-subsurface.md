---
title: "Smithay accepts a second wl_subsurface for a subsurface whose parent was destroyed"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# A second `wl_subsurface` for an orphaned subsurface is not `bad_surface`

Filed 2026-09-23 while implementing the
[subsurface depth bound](../resolved/subsurface-depth-bound-done.md), from
reading the pinned source.

`wl_subcompositor.get_subsurface` says the to-be subsurface "must not have
an existing `wl_subsurface` object. Otherwise the `bad_surface` protocol
error is raised." The pinned Smithay (`src/wayland/compositor/tree.rs`,
`set_parent`) checks for a *parent* instead. When a subsurface's parent
`wl_surface` is destroyed, `cleanup` clears the child's parent while its
`wl_subsurface` lives on, so a second `get_subsurface` for it is accepted:
the surface then has two `wl_subsurface` objects, and destroying the older
one runs `unset_parent`, detaching it from the parent the newer one gave it.

Not a crash and not a depth path: every link still goes through scoot's
depth guard (`subsurface_depth.rs`; the orphan case has its own test). What
it is: one protocol rule not enforced, and a surface whose position in the
tree can change under a `wl_subsurface` the client believes is current. A
fix would track the live `wl_subsurface` per surface in scoot (the guard in
`dispatch.rs` already sees every `get_subsurface`) and refuse with
`bad_surface`; the protocol ties the role object's lifetime to the
`wl_subsurface`'s destructor, which scoot's blanket `destroyed` also sees.
No real client measured does this.
