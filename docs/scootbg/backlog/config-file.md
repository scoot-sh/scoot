---
title: "A config file with per-output defaults (from config-and-rotation)"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# A config file with per-output defaults (from config-and-rotation)

Split 2026-10-08 from
[config-and-rotation](resolved/config-and-rotation-done.md), whose slideshow half ships
in the resolving PR and whose config-file half the ticket itself defers
("Only if people ask; the CLI plus an autostart line may be enough").
Serves **daily-drive**: a static machine whose wallpaper never changes
needs no daemon flags at all.

## The gap

For use outside scoot (inside scoot, `[wallpaper]` in scoot's config
covers this): there is no `~/.config/scootbg/config.toml`. A user who
wants per-output defaults, fit mode, filter and transition settings
without scoot must pass them on every `scootbg set`, or wrap an autostart
line. Nothing is broken: the CLI works without it.

## What to do

Only if people ask. If it happens: `~/.config/scootbg/config.toml` with
per-output defaults, fit mode, filter and transition settings, read once
at daemon start (and re-read on SIGHUP or not at all — decide then). The
CLI keeps working without it and overrides the file. Constraints from the
parent ticket: no `toml` dependency past measurement (`toml` costs +180 KB
over the hand-written line format's +20 KB — see
[lightest](lightest.md); re-measure before adding any parser), and the
file never changes what `apply-config` sends.

## Not in this ticket

The slideshow (`scootbg set DIR --every`, shipped with the parent
ticket): its per-invocation flags already cover mode, fill, filter and
transition per rotation.
