---
title: "Pointer input and interactions: hit-testing, hover, click and scroll actions"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock"
---

# Pointer input and interactions

Filed 2026-09-29. Serves **daily-drive** (volume scroll, launcher and power
buttons) and **computer use** (every interaction is scriptable).

## What to do

- `wl_pointer` on the bar surface: enter, motion, button, and axis (use
  `axis_value120` / discrete steps so a smooth-scroll touchpad and a wheel
  both move a volume step sensibly).
- Hit-test against per-module rects recorded at layout (a pooled `Vec`
  rebuilt only when layout changes); hover changes damage only the affected
  module's rect.
- `on_input(Input) -> Option<Action>` on the module trait. Config keys on
  every module: `on-click`, `on-right-click`, `on-middle-click`,
  `on-scroll-up`, `on-scroll-down`. A value is a module-defined action
  (`"toggle-mute"`) or `{ exec = [...] }`; modules ship defaults.
- A `scoot = "quit"` action kind that talks to scoot's IPC through an
  optional Cargo feature, so log out needs no process spawn and the bar builds
  without scoot's IPC crate. Measure `scoot-ipc`'s cost first (scootbg avoided
  it). Absent scoot, the action reports it cannot run and does nothing.
- No keyboard interactivity: the bar never takes the keyboard, so it never
  disturbs focus.
- Spawned commands are reaped (scoot's own `spawned-children-never-reaped`
  is the precedent) and never inherit the bar's fds.

## Edge cases to pin

Click during a redraw, a module that disappears under the pointer, scroll
flood at a real device's rate (coalesce to one action per frame), a button
release after the pointer left, a touch device (ignored, stated).

## Done when

Clicks and scrolls run the configured action, hover repaints one rect, and a
scroll flood does not spawn a process per event.
