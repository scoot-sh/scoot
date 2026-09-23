---
title: "Many desynchronized subsurfaces in one window stall the compositor roughly quadratically"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Subsurface commits cost in proportion to the whole tree

Filed 2026-09-23 while implementing the
[subsurface depth bound](../resolved/subsurface-depth-bound-done.md), from a
measurement; pre-existing, and not a depth problem -- the depth bound does
not touch it. The subsurface analogue of
[popup-count-quadratic](./popup-count-quadratic.md): not a crash, a stall
any client can cause.

## Measured

A client maps a window, then in one batch creates `N` sibling subsurfaces
of it -- each one level deep, desynchronized, with a 4x4 buffer, committed
-- and commits the window (the `popup_parent/tests` client's
`SubOp::Chain { len: 1 }`, `N` times; a throwaway probe on the
subsurface-depth-bound branch, not committed). Time for the batch to be
dispatched and answered, dev VM:

| N | release | debug |
|---|---|---|
| 1000 | 28 ms | 178 ms |
| 3000 | 84 ms | 1.36 s |
| 10000 | 1.15 s | -- |
| 30000 | over 10 s (harness timeout) | -- |

The same with *synchronized* siblings is not quadratic: 3000 took 38 ms
in release, 197 ms in debug. A frame with 10000 siblings mapped costs
8.5 ms in release (linear, as expected).

## Suspected cause (not verified)

Each desynchronized commit is applied at once, and scoot's commit handler
then walks to the root and calls `Window::on_commit`, which recomputes the
window's bounding box over its whole surface tree -- linear in the tree per
commit, so quadratic over a batch. A synchronized child's commit is only
cached until its parent's commit, which is why that case is cheap. Confirm
with a profile before fixing; a fix would coalesce the per-commit window
work to once per dispatch.
