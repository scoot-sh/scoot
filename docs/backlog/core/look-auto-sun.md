---
title: "look auto --sun: switch looks at sunrise and sunset"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# look auto --sun: switch looks at sunrise and sunset

Filed 2026-11-06 from [`look-auto-switch`](look-auto-switch.md), which
landed `--schedule` only. Serves **daily-drive**, opt-in.

## The gap

`scoot look auto --light A --dark B --sun` was in the original ticket
as "the night-light slot's location settings". Those settings exist only
as Nix options (`programs.scoot.desktop.nightlight.latitude` and
`longitude`, `nix/modules/nightlight-home.nix`), baked into the wlsunset or
gammastep command line in a wrapper script. Nothing at runtime exposes
them, so `scoot` has no latitude to read, and a non-Nix user has none at
all. A systemd `OnCalendar=` cannot say "sunset" either.

## What to do

1. Decide where the location lives at runtime. Candidates: a
   `[location]` the CLI takes (`--latitude`/`--longitude`, stored in the
   unit like `--schedule` is), or the module writing the night-light
   pair to a file beside `look-default` that `auto` reads. Do not parse
   the wrapper script.
2. Compute sunrise and sunset for a date (a small solar-position function
   in `scoot-look`, no daemon, no network), including polar day and night
   (no sunrise or no sunset that day: stay on the previous side).
3. The timer cannot name a moving time. Either a daily timer shortly after
   midnight that installs one-shot `OnCalendar=` transient timers for that
   day's two boundaries, or a self-rearming service. Reuse the
   `_auto-fire` decision: the service picks the side from the clock, so
   catch-up runs still agree.
4. Keep the own-if-marked unit rule, `--off` removing everything, and the
   refusal at `auto` time for a missing look.

## Not in this ticket

`--follow-portal` ([`look-auto-follow-portal`](../resolved/look-auto-follow-portal-done.md)).
