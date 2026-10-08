---
title: "Unprivileged --tty display stays dead after the seat daemon loses the VT-switch-back master race"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Unprivileged --tty display stays dead after the seat daemon loses the VT-switch-back master race

Filed 2026-10-08 from PR #530's review (vtback3 round). Serves
**daily-drive**: a VT switch is the one recovery path back to a running
session, and today an unprivileged session whose race is lost stays dark
until the user retries or restarts.

## The gap

PR #530 measured, live on the Asahi M2, why an unprivileged scoot
`--tty` session cannot recover its own display after a VT switch back --
and why nothing scoot-side can fix it without one of the changes below:

- The kernel gates `SET_MASTER` on **privilege, not vacancy**. An
  unprivileged probe of a genuinely vacant master (all `master n` in
  `debugfs dri/2/clients`, read seconds after) fails with `EACCES`, 7/7
  attempts across two sessions; a root probe of the same vacant master
  takes it and drops it cleanly. So scoot-as-steve can never take master
  back itself, even when nobody holds it.
- seatd gets exactly one re-acquire attempt on the switch back, races
  logind's release of the previous VT's master, and loses (`seat.c:516
  ... Device or resource busy`), then logs `Opened client 2` and never
  retries -- handing the client a master-less fd for the rest of the
  session.
- The resulting state (shipped diagnosis in #530,
  `Tty::reacquire_master` in `crates/scoot/src/compositor/tty/mod.rs`):
  `active` stays `false`, every output reports `live: false` over IPC,
  the session stays alive (keyboard and `scoot msg` answer). Root-run
  sessions self-recover through the probe-plus-retry arm; unprivileged
  ones do not.

The way back documented today (see
`site/src/content/docs/scoot/backends.md#hotplug-vt-switching-captures`):
switch VTs away and back -- each return is a new race the seat daemon
may win (observed: a fresh switch back recovered once in the pre-fix
A/B) -- or restart the session, since a fresh start re-acquires master
through the seat daemon. Full background in
`docs/backlog/resolved/vt-switch-back-greeter-master-done.md`.

## What to do

Pick one of the candidate fixes and pin the edges below:

- **Reopen the device through libseat and rebuild.** On the
  vacant-but-untakeable diagnosis, close the DRM node and re-open it
  through the session (a fresh seatd open is first-to-open while vacant
  and takes master), then rebuild the `DrmDevice`, its surfaces and the
  scanout/presenter state around the new fd. Careful with in-flight
  flips, CRTC state the other VT may have reconfigured, and the
  `stale_vblanks` bookkeeping `reactivate` already clears.
- **Seat-side retry.** Teach seatd to retry the re-acquire after losing
  the race (bounded, a little after the VT activation, not a spin).
  seatd is a dependency, so this goes through the fork policy in
  `CLAUDE.md` (scoot-sh fork, last resort, listed in `dev/forks.md`
  with the rejected alternatives) -- or wait on upstream.
- **A privileged helper.** A helper that takes master on the session's
  behalf. Check the pid gate first: `drm_master_check_perm` wants
  `file->pid == current->tgid`, so a *separate* process likely cannot
  `SET_MASTER` our fd at all -- the realistic privileged shape may just
  be "run the session with privilege", which already recovers today.

Edges to pin in any fix: the held-vs-vacant discrimination (`EBUSY`
vs `EACCES` in the probe) keeps diagnosing correctly; the
privileged-session self-recovery does not regress; outputs report
`live: false` until master is really back; retries are event-driven
(one attempt per `ActivateSession`, never a spin); headless tests pin
the wording, live VT cycles prove the recovery.

## Not in this ticket

The loud message, the `live: false` reporting and the documented retry /
restart way back -- all shipped in #530 and staying regardless. Also
out: Smithay's own pause/activate master handling (deliberately skipped
on the libseat path), and devices that never had master (startup
failures take the `gpu::unusable_device_error` path, a different ticket).
