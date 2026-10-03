---
title: "Tray: coalesce a runaway item's redraws"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Tray: coalesce a runaway item's redraws

Filed 2026-10-03, split out of [tray](resolved/tray-done.md) when its menus landed.
Serves **daily-drive**: a buggy app should not cost the bar a redraw
per announcement.

## The gap

One item re-announcing its icon continuously is re-read at most every
50 ms (measured 0.7% of a core in
[lightest.md](lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02)),
but every re-read still redraws: nothing coalesces redraws across
items, and a menu flooding `LayoutUpdated` re-fills its popup per
floor window the same way.

## What to do

- Coalesce per-turn: a runaway item's announcements within one poll
  turn produce one re-read and one redraw, however many signals
  arrived (the floor stays as the backstop across turns).
- Measure before/after on the flood harness
  (`docs/scootbar/bench/m6-tray-vm/scripts/flood.sh`): reads, CPU
  seconds and wakeups per second for one runaway item, plus a menu
  flooding `LayoutUpdated` while open.
- Hold the idle rows: zero wakeups with a quiet bus, in the same
  table.

## Not in this ticket

The floor itself (stays); menus ([tray](resolved/tray-done.md) landed them).
