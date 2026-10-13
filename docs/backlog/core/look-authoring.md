---
title: "Authoring and sharing: look new, watch, save, install, export"
status: "open"
area: "core"
priority: "medium"
blocked: "look-apply-cli"
---

# Authoring and sharing: look new, watch, save, install, export

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**: new looks should be cheap to make and to share.

## The gap

Making a look today means editing `desktop.nix`, mirroring pins in
`nix/tests.nix` and two docs tables (`site/.../scoot/theming.md:191-210`).

## What to do

The design doc's §8:

- `scoot look new NAME --from BASE` (a directory with `extends`).
- `scoot look watch DIR`: re-validate and re-apply on save, debounced on
  `close_write`/`moved_to`; an invalid save is refused loudly and the last
  good look stays.
- `scoot look save NAME`: the user's own appearance values as tokens.
- `scoot look install PATH|GIT-URL [--trust]`: validate, then copy into the
  catalog; `apps/` static leaves refused without `--trust` and printed (foot,
  starship, Helix and btop configs can run commands); `license` required.
- `scoot look export NAME`.
- `scoot look preview --capture`: a `preview.png` from a headless session.
- Docs: a "make a look" page replacing `theming.md:191-219`.

## Evidence to record

Tests for each refusal (`--trust`, missing license, a command key), and a
watch session recorded: three edits, one invalid, the screen following.

## Not in this ticket

A central registry (Decision D6: none).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
