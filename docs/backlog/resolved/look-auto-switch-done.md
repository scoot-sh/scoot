---
title: "Opt-in automatic look switching: a schedule, sunset, or the portal"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "later"
resolved: "2026-11-07"
---

# Opt-in automatic look switching: a schedule, sunset, or the portal

Filed 2026-10-12, child of [`live-looks`](live-looks.md). **Later**: filed so
the design has a home, not scheduled. Serves **daily-drive**, opt-in.

## What to do (when scheduled)

`scoot look auto --light A --dark B` with `--schedule HH:MM/HH:MM`, `--sun`
(the night-light slot's location settings) or `--follow-portal`, as systemd
user timers or a path unit, never a daemon; off by default; a swap's own
`color-scheme` write must not loop back into `--follow-portal`.

## Status (2026-11-07)

`--follow-portal` landed too (one trigger at a time; see the resolved
`look-auto-follow-portal`). `--schedule` landed: `scoot look auto --light
A --dark B --schedule HH:MM/HH:MM`, bare `auto` for status and `auto --off`, as one systemd user
timer and a oneshot service (`crates/scootctl/src/look/auto/`). The
ticket stays open for the remaining trigger, split out because it is
its own design problem:

- `--sun`: done, [`look-auto-sun`](../resolved/look-auto-sun-done.md)
- `--follow-portal`: done, [`look-auto-follow-portal`](../resolved/look-auto-follow-portal-done.md)

`--sun` landed 2026-11-07 ([`look-auto-sun`](../resolved/look-auto-sun-done.md)): all three triggers are done.

Decisions this ticket left open, settled by the first slice: the units are
one timer with two `OnCalendar=` lines and one service that reads the wall
clock when it fires (two per-look `apply` units would race after a suspend
spanning both boundaries); the unit directory is
`$XDG_CONFIG_HOME/systemd/user`, written atomically, own-if-absent-or-marked;
the timer hangs off `timers.target`; a look missing from the catalog
refuses at `auto` time.

## Not in this ticket

Per-output looks.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
