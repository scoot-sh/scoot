---
title: "The session launcher's 1 s poll is the desktop's biggest idle wakeup source"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# The session launcher's 1 s poll is the desktop's biggest idle wakeup source

Filed 2026-10-05 from a measured idle session on the Asahi M2 (scoot-test,
vinyl look, greetd login, 90 s settle, 60 s window, `/proc/<pid>/stat` and
`status`). Serves **daily-drive** (battery life: the maintainer wants maximum).

## The gap

Over 60 idle seconds the `resources/scoot-session` launcher (`sh`) made 179
context switches and the user manager (`systemd --user`) 178, with 4 + 25 clock
ticks of CPU: the launcher's once-a-second `systemctl --user show -p
ActiveState` poll (its "1 s session poll", see the launcher header and
`docs/nix.md`), each poll a fork/exec plus a manager round trip. Everything
scoot itself runs was quieter: the compositor 31 wakes and 0 ticks, scootbg 0,
scootbar 57 and 1 tick, swayidle 2.

## What to do

Replace the poll with a blocking wait on the service leaving the active state
(e.g. `systemctl --user` job/unit wait mechanisms, `busctl --user wait` /
`monitor` on the unit's `PropertiesChanged` scoped to `scoot.service`, or a
pidfd on the main PID), keeping every behavior #416/#425/#431 pinned: liveness
flock, re-exec survival (a manager re-exec must not read as the session
ending), the silence limit, cleanup and refuse-if-active. If a blocking wait
cannot keep the re-exec semantics, lengthen the poll and say what it costs in
logout latency. Measure wakes before and after the same way.

## Not in this ticket

scootbar's 57 wakes/min (check what fires once a second with a minute clock)
and the compositor's 31/min: worth a look, file separately if real.
