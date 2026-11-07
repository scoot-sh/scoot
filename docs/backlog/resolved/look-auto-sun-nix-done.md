---
title: "Nix: pass the night-light latitude and longitude to look auto --sun"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# Nix: pass the night-light latitude and longitude to look auto --sun

Filed 2026-11-07, split out of [`look-auto-sun`](../resolved/look-auto-sun-done.md).
Serves **daily-drive**, opt-in.

## The gap

`scoot look auto --sun` takes its place from `--latitude` and `--longitude`.
A Nix user who already set `programs.scoot.desktop.nightlight.latitude` and
`longitude` (`nix/modules/nightlight-home.nix`) types the same two numbers
again. No Nix module installs a `scoot look auto` unit at all today
(`grep -r "look auto" nix` is empty): `auto` is imperative, writes
`~/.config/systemd/user`, and refuses a unit it did not write, so a
home-manager-declared `scoot-look-auto.*` would collide with it.

## What to do

Decide whether the module should declare the units (a
`programs.scoot.look.auto` option with `light`, `dark` and a trigger,
reusing the night-light pair for `--sun`) or whether a note in the
night-light docs pointing at the two flags is enough. Declaring them means
the module writes the service and timer text the CLI writes (the
`scoot-look-auto.timer` text is fixed; the service line carries the place),
and `auto --off` must keep leaving a home-manager link alone, which it
already does. The one-shot `scoot-look-auto-next.timer` is runtime state
the service writes, so the module does not declare it.

## Not in this ticket

Reading the wrapper script, a network lookup, or a location service.

## Resolution

Took the second option: a note in the night-light page
(`site/src/content/docs/desktop/night-light.md`) says `look auto --sun` has
its own `--latitude` and `--longitude` and does not read
`desktop.nightlight.latitude`. No module declares the units: the CLI's
own-if-marked rule would collide with a home-manager link, and the saving is
two numbers typed once. Reopen if people ask for a declarative
`programs.scoot.look.auto`.
