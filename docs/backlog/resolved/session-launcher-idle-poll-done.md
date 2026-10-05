---
title: "The session launcher's 1 s poll is the desktop's biggest idle wakeup source"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
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

## Resolved by PR #453 (2026-10-05)

- The session wait blocks in `busctl --user wait` on `scoot.service`'s
  `PropertiesChanged` and re-asks the manager only on a wake
  (`resources/scoot-session`). Subscribe-before-check (spawn the waiter,
  then ask) so a stop between the two still wakes; `timeout 300` bounds
  the residual sliver. Silence limit, liveness flock, refuse-if-active,
  session-target ordering, cleanup/restore and the deadline path are
  unchanged; a loud 1 s poll fallback (re-resolved every round, T13)
  covers no-busctl and pre-`wait` systems.
- Measured on the Asahi M2 (scoot-test, vinyl look, greetd login, 90 s
  settle, 60 s `sample.sh` window, ticks + context switches): launcher
  `sh` 4 ticks + 221 wakes and `systemd --user` 32 ticks + 470 wakes
  before, **0 + 0 and 0 + 0 after** — the session went from 0.60% of a
  core to unmeasurable. Logout ends the session in 0.10 s; a real
  `nh os switch` mid-session (manager re-exec, no unit changes) leaves
  the launcher and its still-blocked waiter untouched with IPC answering.
  Evidence: `before-poll-delta.tsv` / `after-block-delta.tsv` beside the
  report.
- Harness 17 → 37 asserts (fake busctl, manager-down re-exec flag, true
  activation semantics, blocking wait, 50-re-exec storm, logout latency,
  carried limitation, both fallbacks, transient-resolve recovery).
