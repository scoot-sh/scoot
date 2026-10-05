---
title: "scootbar wakes about 40 times a minute at idle"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootbar wakes about 40 times a minute at idle

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`docs/benchmarks.md`), answering the open question in
`docs/backlog/core/session-launcher-idle-poll.md` ("file separately if
real"). Serves **daily-drive** (battery: wakeups are the idle-CPU
story once the launcher poll is fixed).

## The gap

It is real: the bar wakes 36-42 times per 60 s at idle with zero ticks
(`ev/scoot-c*-delta.tsv`, `vol=` column for `scootbar daemon`). The
likely source is the clock module ticking once a second to render a
minute-resolution clock (`clock.format = "%-I:%M %P"` in the measured
config) — waybar does the same (64-68 wakes/min with the same content),
so this is the going rate for a 1-second clock tick, not a scootbar
defect per se.

Scale: after the launcher-poll fix lands, the session's remaining
wakeups are roughly the manager (~200/min, also poll-driven today),
the bar (~40) and the compositor (~15). The bar would then be the
second-biggest waker.

## What to do

Confirm the source (strace the wakes for a minute, or gate the clock
tick behind "does the rendered minute change"), then wake the clock
only when its text changes (expected saving: ~35 wakes/min, from ~40
to the DND/unread feed's handful). Keep 1-second ticks for formats
that show seconds. Edge cases: DND toggle and notification count must
still update promptly (they are event-driven already); timezone
changes and suspend/resume must re-render.

## Not in this ticket

The launcher poll and the manager wakeups (that ticket); waybar's
higher rate (competitor observation, not ours); the bar's 4.6 MB PSS
(already 7-9x lighter than waybar — see the benchmark).
