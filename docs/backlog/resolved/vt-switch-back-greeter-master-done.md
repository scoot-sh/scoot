---
title: "Switching back to a scoot --tty session fails to reactivate DRM while the greeter holds master"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Switching back to a scoot --tty session fails to reactivate DRM while the greeter holds master

Filed 2026-10-07. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Filed 2026-10-07 from the #492 round: the implementer saw it live on the
Asahi M2, and the reviewer confirmed it is not the same defect as
[05b-vt-switch-eperm](../../roadmap/05b-vt-switch-eperm.md). Serves
**daily-drive**: a VT switch is the one recovery path back to a running session.

## The gap

On the M2 a `--tty` scoot started on VT2 (private seatd + `openvt`) loses
its display after `chvt 1` then `chvt 2`. The log shows `session activated`,
then `could not reactivate the drm device ... Failed to disable connectors ...
Permission denied (os error 13)`. The greeter (cage + regreet, uid 998, on tty1
since login, holding authenticated DRM master through logind) keeps master,
and the scoot session on its own seat cannot take it back.

05b was `change_vt` failing while paused inside libseat (fixed through
`session_paused`). This is `drm.activate` / disabling connectors failing after
the seat reports active. It fails before any of the lit-gate logic runs.

## What to do

- Reproduce on the M2 with the private seatd + `openvt` pattern from
  `docs/backlog/resolved/seatd-loss-panics-done.md`, with a greeter holding
  master on tty1, and with no greeter (plain getty) to see whether the
  greeter is required.
- Find who holds master at the time of the failure (`debugfs`
  `dri/N/state`, `fuser`, the seat daemons' logs), and whether logind or
  seatd arbitrates it. Check what `DRM_IOCTL_SET_MASTER` returns and whether
  scoot should retry, drop-then-set master explicitly, or report a clear error.
- Decide the expected behavior when master cannot be had: retry with a
  bound, then a visible message and a way back, never a silent black screen.
- A test that fails before the fix if the activate path can be driven
  against a fake device; otherwise a recorded live repro.

## Resolution (2026-10-08, PR #530)

Diagnosed live on the M2 (round 1, recorded in the coordinator's
scratchpad): seatd's single re-acquire attempt on switch-back races
logind's release and loses (`seat.c:516 ... Device or resource busy`),
hands the client a master-less fd, never retries; scoot's
`reactivate()` → `reset_state()` then fails `EACCES`, `Tty.active`
stays false, no render is requested. A bounded retry of `activate()`
cannot fix it (nothing re-acquires master).

Round 2 measured what `SET_MASTER` from scoot's own fd actually returns
(private seatd + `openvt`, VT2, three `chvt` cycles): on a genuinely
vacant master (all `master n` in `debugfs dri/2/clients`) an
unprivileged process gets `EACCES` -- the kernel gates `SET_MASTER` on
privilege, not vacancy (a root probe takes and drops the same vacant
master cleanly). So scoot-as-steve can never steal master back itself;
`EBUSY` from the probe means genuinely held (usually the greeter),
`EACCES` means vacant-but-untakeable (seatd lost the race).

Shipped accordingly, all in scoot (no Smithay fork change -- nothing
private to Smithay blocks the probe ioctl):

- `Tty::reacquire_master` (`crates/scoot/src/compositor/tty/mod.rs`):
  on a master-loss-shaped `activate` failure (`Access` + `EACCES`) it
  tries one explicit `SET_MASTER` (recovers privileged/root-run
  sessions outright) and retries the activation, then logs loudly and
  precisely -- holder vs vacancy -- with the way back (session alive,
  retry on the next switch back). `active`/`session_paused` semantics
  unchanged (`change_vt` still gated on `session_paused` only, per 05b).
- `scoot msg outputs` gains a defaulted `live` field per output
  (`crates/scoot-ipc`, no `PROTOCOL_VERSION` bump per the `powered`
  precedent): `false` while `--tty` has lost master. Screenshots go
  stale while dead -- check `live` before trusting pixels.
- Headless unit tests pin the guidance wording (both variants) and the
  steal gate; the ioctl half is proven by the recorded live A/B below.
- Docs: `site/.../scoot/backends.md` VT-switching section + symptom
  box, `troubleshooting.md` symptom, `msg/requests.md` `live` row,
  CHANGELOG entry.

Proof: before/after live repros on the M2 (same pattern, this branch's
own debug builds): before fails 2/3 cycles with the exact ticket error;
after logs the vacancy diagnosis with `live: false` over IPC on every
failure, session alive, greeter untouched. Full recovery of an
unprivileged session needs seatd to re-acquire (or hand over) master --
left as a maintainer finding: seatd-as-root *can* take the vacant
master (proven), it just never retries after losing the race.
