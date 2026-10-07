---
title: "Switching back to a scoot --tty session fails to reactivate DRM while the greeter holds master"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
