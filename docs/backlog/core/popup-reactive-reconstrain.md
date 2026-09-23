---
title: "A reactive popup is not re-constrained when its parent moves or its output changes"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Reactive popup re-constraining

Filed 2026-09-23 while landing
[popup constraint adjustment](../resolved/popup-constraint-adjustment-done.md),
which constrains a popup against its output at the two points the client
is told its geometry: the initial configure and `xdg_popup.reposition`.
Serves **daily-drive** (a menu left open while its column scrolls).

## What is wrong

`xdg_positioner.set_reactive` (v3) asks that "the surface is reconstrained
if the conditions used for constraining changed, e.g. the parent window
moved", answered with a fresh `xdg_popup.configure` + `xdg_surface.configure`.
scoot never re-constrains: a popup keeps the geometry it was configured
with while its parent scrolls with its column, the output is resized, or a
bar's exclusive zone changes -- so a menu slid inside the output at open
time can end up cut again. A non-reactive popup is correct as it is (the
protocol forbids re-configuring it).

In practice the window is small: opening a menu takes a popup grab, and the
things that scroll a column (focus changes, clicks elsewhere) mostly dismiss
it first. An IPC-driven scroll while a menu is open, or a hotplug, is what
would show it.

## What to do

After `apply()` (and on output/usable-area changes), for each tracked xdg
popup whose committed positioner is `reactive`, recompute
`State::constrained_popup_geometry` (`popup_constraint.rs`) and, when it
differs from the committed geometry, set it pending and `send_configure()`
(which Smithay permits exactly for a reactive positioner). This must not cost
the common `apply()` anything when no popup is open: gate it on the popup
manager having any mapped popups. Harness test: open a reactive popup with
`SlideX` near the right edge, scroll its column left by an IPC action, and
assert a second configure arrives with the re-slid position; a non-reactive
twin gets none.
