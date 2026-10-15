---
title: "Reload wallpaper wait as a deferred reply: park the IPC reply, answer from the reaper or a timer"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Reload wallpaper wait as a deferred reply: park the IPC reply, answer from the reaper or a timer

Filed 2026-10-15, split out of the look-layer-scoot review (#613). Serves
**daily-drive**: a user or agent that asks "did my wallpaper land" needs an
answer without freezing the session to get it.

## The gap

`scoot msg reload --wait-wallpaper` blocked the event-loop thread with
`thread::sleep` polling (`reload.rs::wait_wallpaper`), freezing the entire
compositor for up to 30 s -- and it always timed out, because scootbg needs
the loop to service its Wayland requests while it works. The flag was
removed from the protocol in #613 (`Request::Reload` is a unit variant
again); the reload reply answers once the wallpaper run is handed over,
without waiting for it. There is no way to ask "tell me when the wallpaper
run exits".

## What to do

Design a deferred reply: park the IPC reply on the reload, answer it from
the `SIGCHLD` reaper when the `scootbg apply-config` run exits (or from a
timer past a bound, reporting handed-over-but-unwaited-on). Pin the edge
cases: the run exits before the park, the client disconnects while parked,
two reloads park two replies, SIGHUP has no reply channel so it never parks.
Never block the loop thread: the park is state plus a wakeup, not a sleep.

## Not in this ticket

Changing what a plain reload applies, or the fire-and-forget default.
