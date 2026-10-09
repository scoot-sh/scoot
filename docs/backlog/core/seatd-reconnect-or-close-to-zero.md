---
title: "Recover the display scoot-side after the seat daemon loses the VT-switch-back master race: seat reconnect or close-to-zero"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Recover the display scoot-side after the seat daemon loses the VT-switch-back master race: seat reconnect or close-to-zero

Filed 2026-10-09 from
[seatd-reacquire-after-vt-switch](../resolved/seatd-reacquire-after-vt-switch-done.md),
whose same-client reopen was tried live on the Asahi M2 and proven
unworkable (see below). Serves **daily-drive**: a VT switch is the one
recovery path back to a running session, and today an unprivileged
session whose race is lost stays dark until the user retries or
restarts.

## Decision (2026-10-09): no seatd fork

User: "I'd prefer we not do another fork on this." Seat-side retry is
off the table -- no scoot-sh seatd fork, no upstream wait. This ticket
covers only the two scoot-side routes below.

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
  Same-client reopen is disproven (see the parent ticket); seat
  reconnect and close-to-zero below remain unproven, not disproven.
- Switch away deactivates (`DROP_MASTER` on the shared file); switch back
  re-activates with one `SET_MASTER` that logs `EBUSY` and continues
  master-less on failure. No retry on any later event.
- Close-to-zero deactivates, closes and frees the entry -- so a
  close-then-open *would* fresh-open and (while vacant) take master. But
  scoot cannot close its DRM fd back to the session today: past
  `DeviceFd::from` it belongs to an `Arc<OwnedFd>` with no way back out,
  and `Session::close` needs the `OwnedFd`. Freeing that up is route (b).

Research (2026-10-09 survey of wlroots/sway, Smithay/niri,
Hyprland/aquamarine, Mutter, KWin, weston, cage, seatd and logind;
strategy only, no code copied from GPL projects): no compositor retries
client-side on the VT-back path -- the pattern is keep-fd plus
survive + report + heal on a later race (delegating master to the
privileged seat manager, re-scanning connectors and modesetting fresh on
the session-active event). logind avoids the race by design: the root
daemon claims master on its own kept-open fd *before* handing the fd /
resume signal to the client, so there is no client-side `SET_MASTER`
race at all. On seatd the two scoot-side routes below are the only ones
that can still yield a master-holding fd while vacant; everything else
(client ioctl retry, same-client reopen, a helper claiming this fd from
another process, path aliases, KWin-style open retry) is dead or
logind/root-only.

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

Either route heals through the proven teardown-first rebuild (old pools
dropped before `DrmDevice::new`); both trigger event-driven -- one
attempt per `ActivateSession` on the vacant-but-untakeable diagnosis,
never a spin -- and both keep the shipped survive+report+heal baseline
(#530: loud message, `live: false`, documented retry/restart way back)
unchanged underneath.

- **(a) Seat reconnect (scoot-side, unproven).** Drop the seat connection
  and reconnect (`LibSeatSession::new`), so the re-open is a *new* client
  entry and seatd fresh-opens (entry was freed on disconnect) and takes
  vacant master while vacant; then rebuild via the proven
  teardown-first path and re-init libinput on the new session. Risks to
  prove first: the new session must come up active (true on a private
  seatd with no other client; likely `EPERM`-shaped under logind, which
  needs its own hardware matrix), the input gap across the swap, and the
  extra event-loop token plumbing. Needs coordinator scope approval --
  bigger blast radius than the reopen the parent ticket tried.
- **(b) Close-to-zero then reopen (scoot-side, needs a refactor).** Regain
  `OwnedFd` ownership (refactor the `Arc<OwnedFd>` holders) so
  `Session::close` can free the entry, then open-then-rebuild in the same
  client. Same master mechanics as (a) (freed entry -> fresh open ->
  seatd claims while vacant) with no second session identity. Risks: the
  refactor touches every DRM-fd holder; in-flight flips/CRTC state plus
  the `stale_vblanks` bookkeeping need the same care as (a); still needs
  the teardown-first ordering. Ranked below (a) only because the refactor
  cost is upfront while (a) can be spiked behind a flag.

Edges to pin in either fix (carried over, plus research): the
held-vs-vacant discrimination (`EBUSY` vs `EACCES` in the probe --
pinned headlessly by `probe_held`); no regression of privileged
self-recovery (unchanged); outputs report `live: false` until master is
truly back; retries event-driven (one per `ActivateSession`, no spin);
headless tests for logic/wording; on every `ActivateSession`, re-probe
and schedule exactly one deferred rescan/repaint hook so a later-won
race lights up without further user action.

Live test matrix (Asahi M2): private-seatd unprivileged VT bounce x N,
greeter-on-tty1 vs no-greeter, plus a logind-present control.

## Not in this ticket

The loud message, the `live: false` reporting and the documented retry /
restart way back -- all shipped in #530 and staying regardless. The
diagnosis they rest on is unchanged by the failed reopen above.
Explicitly not pursued: seat-side retry (declined, see the decision
above); client-side `SET_MASTER` retry loops (futile unprivileged,
`EACCES`); same-client reopen (disproven in the parent ticket);
a privileged helper claiming this session's fd from another process (pid
gate); second-client / path-alias games (`EPERM` / `realpath`); or
spinning/polling of any kind.
