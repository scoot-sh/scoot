---
title: "Unprivileged --tty display stays dead after the seat daemon loses the VT-switch-back master race"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-09"
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

## Resolution (2026-10-09, no behavior change)

Worked as specified -- the reopen was implemented, measured live, and
disproven -- so this resolves with evidence and a kept pin rather than a
recovery.

Tried first: the full scoot-side reopen (close the master-less device,
re-open through the session, rebuild the `DrmDevice`, its heads and the
event-loop wiring, re-bind outputs). Live on the Asahi M2 it triggers
correctly on the vacant diagnosis and then fails deterministically: the
fresh `session.open` succeeds and probes fine, but the rebuilt device
has no master either. Mechanism, checked against seatd 0.9.3's source
(`seat.c`: `seat_open_device` reuses the entry for an already-open
(client, path) with a bumped refcount -- no fresh `open()`, no new
`SET_MASTER`; `seat_activate_device` does the single `SET_MASTER`,
logs `EBUSY` and continues master-less; only close-to-zero frees the
entry): a same-client re-open hands back the *same* master-less file,
so no same-client reopen can ever take master. That disproves only the
same-client reopen: seat reconnect (a new libseat client, whose open is
first-to-open on the freed entry) and close-to-zero-then-reopen remain
unproven, not disproven. Close-then-open in the same client is
API-blocked today (the fd
belongs to an `Arc<OwnedFd>` past `DeviceFd::from`; `Session::close`
needs the `OwnedFd`), a second client is refused while one is active
(`EPERM`), and path aliases canonicalize to the same entry.

Detour of lasting value, measured on the way (`strace`-counted):
rebuilding with the old pools alive fails `DrmDevice::new`'s
framebuffer snapshot (`MODE_OBJ_GETPROPERTIES` `EINVAL` on our own dumb
framebuffer: 2 connectors + 2 CRTCs + 2 foreign framebuffers read fine
first); after a full teardown the same build passes the snapshot. So a
teardown-first rebuild is proven viable -- only the master never
arrives. That path is reusable if a future route (seat reconnect, see
the follow-up) produces a master-holding fd.

Kept pin: `probe_held` in `crates/scoot/src/compositor/tty/mod.rs` with
its headless test -- the `EBUSY`-vs-`EACCES` discrimination every
re-acquire path hinges on. Everything else from the spike is reverted;
`git log` on the branch shows the whole arc.

What remains is tracked, not just prose: see
[Recover the display scoot-side after the seat daemon loses the VT-switch-back master race: seat reconnect or close-to-zero](./seatd-reconnect-or-close-to-zero-done.md)
(medium, unblocked; per the 2026-10-09 user decision there is no seatd
fork, so seat-side retry is off the table) -- seat reconnect as a fresh
libseat client, or the close-to-zero refactor, each with the mechanism,
the measurements and the edges to pin. That ticket also records the
2026-10-09 cross-compositor research: no compositor retries client-side
on the VT-back path (keep-fd plus survive + report + heal is the
pattern), and logind avoids the race by claiming master before handing
the fd back.
