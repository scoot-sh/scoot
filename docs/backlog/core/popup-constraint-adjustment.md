---
title: "Popups are placed exactly where the positioner says: no constraint adjustment against the parent's output"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Popup constraint adjustment

Filed 2026-09-23 while fixing
[windows bleeding across outputs](../resolved/windows-bleed-across-outputs-done.md).
Serves **daily-drive** (menus near a screen edge) and **computer use** (an
agent reading a menu from a screenshot sees the whole menu).

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
output the parent is placed on (`output_clip::placed_on`), in the parent's
coordinates -- with the pinned rev's
`PositionerState::get_unconstrained_geometry(target)`
(`wayland/shell/xdg/mod.rs`), at `new_popup` and on `reposition_request`.
Decide whether the target is the whole output or the usable area (niri-style
compositors keep menus off an exclusive-zone bar; check the protocol text
and at least one real client, e.g. foot's or a GTK context menu, rather
than assuming). Layer-surface popups (a bar's dropdown) need the same
against their layer's output. Fail-first harness tests: a popup that would
cross the right edge flips or slides back inside, one at a shared edge
stays on its parent's output, and one with no adjustment flags stays cut.
