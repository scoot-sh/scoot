---
title: "Opt-in automatic look switching: a schedule, sunset, or the portal"
status: "open"
area: "core"
priority: "low"
blocked: null
milestone: "later"
---

# Opt-in automatic look switching: a schedule, sunset, or the portal

Filed 2026-10-12, child of [`live-looks`](live-looks.md). **Later**: filed so
the design has a home, not scheduled. Serves **daily-drive**, opt-in.

## What to do (when scheduled)

`scoot look auto --light A --dark B` with `--schedule HH:MM/HH:MM`, `--sun`
(the night-light slot's location settings) or `--follow-portal`, as systemd
user timers or a path unit, never a daemon; off by default; a swap's own
`color-scheme` write must not loop back into `--follow-portal`.

## Not in this ticket

Per-output looks.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
