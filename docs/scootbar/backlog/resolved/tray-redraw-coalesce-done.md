---
title: "Tray: coalesce a runaway item's redraws"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-07"
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

## Status (2026-10-08): done in PR #515

Content changes past the first after a quiet spell — an item's new
icon or title, a menu's new rows — wait out a 100 ms draw gap
(`DRAW_GAP`, the media/bluetooth/window-title pattern) instead of
redrawing with every re-read: one re-read and one redraw per turn
however many signals arrived, ten redraws a second at most across
turns. Appearing, emptying and mode changes still draw at once; the
50 ms re-read floor is untouched, and the draw timer exists only while
a change is held, so a quiet bus still wakes nothing.

Measured on the Asahi M2 (release builds, headless scoot, private
dbus-daemon, `jeepney` peers; loads beside every number in
[lightest.md](lightest.md#m6-tray-redraw-coalescing-measured-2026-10-08)):
icon flood ~445 reads in 20 s both sides (the floor), redraws 444
against 442 reads on base to 233 on the branch (~22 to ~12 a second;
wakeups flat at ~60 to 72 a second, the signal-plus-reply traffic
dominates); menu flood ~400 layouts both sides, ~217 to 222 wakeups
both; idle tray alone 0 wakeups in 60 s, tray with a clock 2 (the
clock's), RSS flat, on both sides. Binary: file +0, `.text` +1,280 B
(+0.07%), `.rodata` +0 — a size-row regression for the maintainer to
waive or not, like the network-child +1,248 B; nothing else regresses
and nothing is claimed waived.

Tests (each fails without the routing, proven by revert-run-restore):
`a_runaway_item_is_redrawn_ten_times_a_second_at_most`,
`a_second_change_inside_the_gap_is_held_and_not_lost`,
`a_flooding_menu_is_redrawn_ten_times_a_second_at_most`.
