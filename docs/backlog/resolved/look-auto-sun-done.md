---
title: "look auto --sun: switch looks at sunrise and sunset"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# look auto --sun: switch looks at sunrise and sunset

Filed 2026-11-06 from [`look-auto-switch`](look-auto-switch-done.md), which
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

## Landed (2026-11-07)

`scoot look auto --light A --dark B --sun --latitude N --longitude E`
(`crates/scootctl/src/look/auto/`, the position in
`crates/scoot-look/src/sun.rs`). Decisions, against the four numbered
items above:

1. **Location.** The CLI takes it (`--latitude`/`--longitude`, plain
   decimals, validated ranges, required with `--sun`) and the service line
   stores it, as `--schedule` stores its times. Held as whole micro-degrees
   so a `Plan` stays `Eq` and the text reads back exactly. The Nix wiring
   is split out: [`look-auto-sun-nix`](../core/look-auto-sun-nix.md).
2. **Computation.** One function, the Sun's altitude (NOAA's low-precision
   position); sunrise and sunset are the instants it crosses -0.833 degrees,
   found by scanning a minute at a time and bisecting to the second. No
   separate sunrise formula, so a polar day or night needs no case: no
   crossing in the span means nothing to arm and the side stays. Tested
   against US Naval Observatory times for New York, London, Sydney, Quito,
   Honolulu, Anchorage, Tromso and Reykjavik, all within 30 s of the printed
   minute, plus the polar day, the polar night and the first sunrise after it.
3. **Timer.** The ticket's second option, with a persistent file timer as
   its safety net: `scoot-look-auto.timer` (00:05 local daily, plus 30 s
   after the user manager starts) and a one-shot
   `scoot-look-auto-next.timer` in `$XDG_RUNTIME_DIR/systemd/user`, written
   by every service run for the next boundary in UTC (`OnCalendar=... UTC`),
   so no time zone or daylight-saving rule is involved. The run picks the
   side from the Sun's position at the moment it runs, so catch-up runs
   agree. Verified on systemd 261.2 (a user manager, `systemd-analyze
   --user verify`, a real boundary fired within a second, a stopped timer
   restored by the next run, polar retire, switching to `--schedule`,
   `--off`).
4. **Rules kept.** Own-if-marked (including the runtime timer), `--off` and
   a switch to another trigger remove everything, a look missing from the
   catalog refuses at `auto` time, unit code shared with the other
   triggers.

Not done: a reboot or logout (the `OnStartupSec=` path) was not exercised,
only emulated by stopping the one-shot timer; a boundary crossed during a
real suspend was not observed.
