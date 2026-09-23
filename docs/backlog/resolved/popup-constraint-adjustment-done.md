---
title: "Popups are placed exactly where the positioner says: no constraint adjustment against the parent's output — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Popup constraint adjustment — RESOLVED

RESOLVED 2026-09-23 (PR #225). An `xdg_popup`'s geometry is flipped, slid
or resized per its positioner's `constraint_adjustment` into a target
rectangle; the rules, and why each, are in
`crates/scoot/src/compositor/popup_constraint.rs`:

- **Target.** A window's popup: its output's usable area (a window's
  popups draw below the `top` layer, so a menu slid under a bar would be
  hidden by it); the whole output while that window covers it fullscreen
  (the `top` layer is not drawn then). A layer surface's popup: the layer's
  whole output. The protocol names the compositor's "work area" as its
  example; a real client (GTK3 3.24.52's entry context menu, under
  `WAYLAND_DEBUG=1`) asks for all six adjustments
  (`set_constraint_adjustment(63)`) and leaves the fitting to the
  compositor entirely -- it has no idea where the output edge is.
- **Coordinates.** Relative to the immediate parent, walking up the
  ancestor popups' committed positions, so a submenu is fitted against the
  edge measured from its parent menu.
- **When.** At the initial configure, not `new_popup` as this entry said:
  a layer surface's popup has no parent yet at `new_popup`, while by its
  first commit it must have one. Also on `xdg_popup.reposition`. Not
  re-done later -- [reactive re-constraining](../core/popup-reactive-reconstrain.md)
  is filed.
- **No target** (parent unmapped, parentless ancestor) or **any input
  beyond 2^24** leaves the positioner's own geometry. The bound is what
  keeps Smithay's `get_unconstrained_geometry` from overflowing `i32`,
  which it does (a debug-build panic, measured) for a positioner near
  `i32::MAX` whose unadjusted geometry is still in range.

Also fixed, found writing the walk: a popup whose parent chain loops back to
itself (`get_popup` naming its own `xdg_surface`, or a two-popup loop
through a bare `xdg_surface`) froze the compositor inside Smithay's
`find_popup_root_surface`, reached from `PopupManager::track_popup`. It is
now refused in `new_popup`, before tracking, with the client disconnected
(`popup_parent.rs`).

## Evidence

Recorded in the PR's description (exact commands, SHAs, raw output):
fail-first runs of the new suite (`popup_constraint/tests/`) against the
handler reverted to `main`, a mutation run per target rule, and the
cycle refusal's hang before the fix. Live, on the dev VM, GTK3's
`gtk3-widget-factory` under `--headless --outputs 2` at 420x360, a context
menu opened at (330, 262) on the first output's right edge: before
(`c9d50cc`) configured at `(319, 251, 149, 165)` and cut at the shared and
bottom edges; after, `(169, 85, 149, 165)` -- flipped on both axes and
whole on its own output, nothing on the second.

---

The original entry follows.

Filed 2026-09-23 while fixing
[windows bleeding across outputs](../resolved/windows-bleed-across-outputs-done.md).
Serves **daily-drive** (menus near a screen edge) and **computer use** (an
agent reading a menu from a screenshot sees the whole menu).

**High since 2026-09-23:** with windows confined to their own output, a
menu crossing a *shared* output edge is now cut off there rather than drawn
onto the neighbour -- and agents running `--headless --outputs N` put
windows beside those edges routinely.

## What is wrong

`XdgShellHandler::new_popup` and `reposition_request` (`handlers.rs`) take
the positioner's geometry as-is (`positioner.get_geometry()`): the
`constraint_adjustment` a client asks for (slide, flip, resize) is never
applied. A menu opened near an output's edge is cut at that edge -- the
framebuffer ends there. Since windows are confined to their own output (a
popup goes with its parent, see `output_clip.rs`), that is now equally true
at the edge *between* two outputs, where before it drew onto the neighbour
(over that screen's windows, taking their clicks).

## What to do

Constrain each xdg popup against its parent window's output rect -- the
output the parent is placed on (`output_clip::placed_on`, which is `Some`
while the parent is mapped -- a popup of an unmapped parent is not drawn
anyway), in the parent's coordinates -- with the pinned rev's
`PositionerState::get_unconstrained_geometry(target)`
(`wayland/shell/xdg/mod.rs`), at `new_popup` and on `reposition_request`.
Decide whether the target is the whole output or the usable area (niri-style
compositors keep menus off an exclusive-zone bar; check the protocol text
and at least one real client, e.g. foot's or a GTK context menu, rather
than assuming). Layer-surface popups (a bar's dropdown) need the same
against their layer's output. Fail-first harness tests: a popup that would
cross the right edge flips or slides back inside, one at a shared edge
stays on its parent's output, and one with no adjustment flags stays cut.
