---
title: "Seat daemon loses the VT-switch-back master race with no retry; unprivileged sessions stay dark"
status: "open"
area: "core"
priority: "medium"
blocked: "maintainer decision: seatd fork vs upstream wait (fork policy in CLAUDE.md)"
---

# Seat daemon loses the VT-switch-back master race with no retry; unprivileged sessions stay dark

Filed 2026-10-09 from
[seatd-reacquire-after-vt-switch](../resolved/seatd-reacquire-after-vt-switch-done.md),
whose scoot-side reopen was tried live on the Asahi M2 and proven
unworkable (see below). Serves **daily-drive**: a VT switch is the one
recovery path back to a running session, and today an unprivileged
session whose race is lost stays dark until the user retries or
restarts.

## The gap

On a VT switch back, seatd re-acquires DRM master for the session with
exactly one `SET_MASTER` and never retries. If logind still holds the
previous VT's master at that instant (`Device or resource busy`), seatd
hands the client a master-less fd anyway (`Opened client N`) and the
unprivileged session stays dark (`live: false`, keyboard and `scoot msg`
alive) until some later switch back wins the race or the session
restarts. Root-run sessions self-recover through scoot's probe arm;
unprivileged ones cannot take master themselves (the kernel gates
`SET_MASTER` on privilege, not vacancy -- measured 7/7 `EACCES` on a
genuinely vacant master).

Mechanism, verified against seatd 0.9.3's source (`seat.c`,
`seat_open_device` / `seat_activate_device` / `seat_close_device` /
`seat_deactivate_device`):

- One open file per (client, path), refcounted. A same-client re-open of
  an already-open path bumps the refcount and returns the *same* file --
  no fresh `open()`, no new `SET_MASTER`. A different path string cannot
  route around it (`realpath()` canonicalizes first) and a second client
  cannot help (`seat_open_device` refuses inactive clients with `EPERM`).
- Switch away deactivates (`DROP_MASTER` on the shared file); switch back
  re-activates with one `SET_MASTER` that logs `EBUSY` and continues
  master-less on failure. No retry on any later event.
- Close-to-zero deactivates, closes and frees the entry -- so a
  close-then-open *would* fresh-open and (while vacant) take master. But
  scoot cannot close its DRM fd back to the session: past
  `DeviceFd::from` it belongs to an `Arc<OwnedFd>` with no way back out,
  and `Session::close` needs the `OwnedFd`.

Live evidence (Asahi M2, private seatd + `openvt`, greeter on tty1):

- Baseline (main): switch back loses the race (`seat.c:516 ... Device
  or resource busy`, `Opened client 2`), scoot logs the vacant diagnosis,
  both outputs `live: false`, debugfs shows no master anywhere. Fresh
  screenshots byte-identical (stale).
- Reopen attempt (spiked, reverted): the fresh `session.open` succeeds
  and its probe reads fine, but the rebuilt device has no master either
  (`Failed to disable connectors` on its first commit) -- the dup-sharing
  above, measured end to end. Detour found on the way: rebuilding with
  the old pools alive fails `DrmDevice::new`'s framebuffer snapshot
  (`MODE_OBJ_GETPROPERTIES` `EINVAL` on our own dumb framebuffer,
  `strace`-counted: 2 connectors + 2 CRTs + 2 foreign framebuffers read
  fine first); after a full teardown the same build passes the snapshot.
  So a teardown-first rebuild is proven viable -- it is only the master
  that never arrives.

## What to do

Pick one, with the fork policy in `CLAUDE.md` (scoot-sh fork, last
resort, listed in `dev/forks.md` with rejected alternatives) for
anything seat-side:

- **Seat-side retry (fork or upstream).** Teach seatd to retry the
  re-acquire after losing the race (bounded, shortly after the VT
  activation, never a spin). Mechanistically sound: seatd-as-root *can*
  `SET_MASTER` its own file any time master is vacant. Needs a trigger
  seatd does not currently have (the switch event already fired), and a
  maintainer decision between carrying a fork and waiting on upstream.
  Never open upstream PRs from here.
- **Seat reconnect (scoot-side, unproven).** Drop the seat connection and
  reconnect (`LibSeatSession::new`), so the re-open is a *new* client
  entry and seatd fresh-opens (entry was freed on disconnect) and takes
  vacant master; then rebuild via the proven teardown-first path and
  re-init libinput on the new session. Risks to prove first: the new
  session must come up active (true on a private seatd with no other
  client; likely `EPERM`-shaped under logind, which needs its own
  hardware matrix), the input gap across the swap, and the extra
  event-loop token plumbing. Needs coordinator scope approval -- bigger
  blast radius than the reopen this ticket's parent tried.
- **Privileged helper.** Rejected: `drm_master_check_perm` wants
  `file->pid == current->tgid`, so a separate process cannot
  `SET_MASTER` this session's fd. The realistic privileged shape stays
  "run the session with privilege", which already recovers today.

Edges to pin in any fix (carried over): the held-vs-vacant
discrimination (`EBUSY` vs `EACCES` in the probe -- pinned headlessly by
`probe_held`); no regression of privileged self-recovery; outputs report
`live: false` until master is truly back; retries event-driven (one per
`ActivateSession`, no spin); headless tests for logic/wording.

## Not in this ticket

The loud message, the `live: false` reporting and the documented retry /
restart way back -- all shipped in #530 and staying regardless. The
diagnosis they rest on is unchanged by the failed reopen above.
