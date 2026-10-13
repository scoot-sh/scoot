---
title: "Opt-in automatic look switching: a schedule, sunset, or the portal"
status: "open"
area: "core"
priority: "low"
blocked: "look-toolkit-live"
---

# Opt-in automatic look switching: a schedule, sunset, or the portal

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**, opt-in.

## The gap

There is no way to follow time of day or an external dark-mode toggle.

## What to do

`scoot look auto --light A --dark B` with `--schedule HH:MM/HH:MM`,
`--sun` (from the night-light slot's location settings) or
`--follow-portal` (when something else sets `org.freedesktop.appearance
color-scheme`). Delivered as systemd user timers or a path unit, never a
new daemon; off by default. Make sure a look swap's own `color-scheme`
write cannot loop back into `--follow-portal`.

## Evidence to record

A timer-driven swap recorded, and the loop guard tested.

## Not in this ticket

Per-output looks.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
