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

## Status (2026-11-06)

`--schedule` landed: `scoot look auto --light A --dark B --schedule
HH:MM/HH:MM`, bare `auto` for status and `auto --off`, as one systemd user
timer and a oneshot service (`crates/scootctl/src/look/auto/`). The
ticket stays open for the other two triggers, split out because each is
its own design problem:

- `--sun`: [`look-auto-sun`](look-auto-sun.md)
- `--follow-portal`: [`look-auto-follow-portal`](look-auto-follow-portal.md)

Resolve this ticket when both are done, or close it once they stand alone.

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
