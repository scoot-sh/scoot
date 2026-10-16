---
title: "scoot look next, prev, random and timed previews"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# scoot look next, prev, random and timed previews

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**.

## The gap

After `look-apply-cli`, a look is applied by name only.

## What to do

- `scoot look next`, `prev`, `random` over the catalog order.
- `scoot look apply NAME --for SECONDS` and `--keep`: the revert records the
  generation it reverts from and does nothing if any later apply moved it;
  it survives the terminal closing (a detached child or a user timer) and
  takes the same lock.
- The default keybinding (D8, `Mod+Shift+T` → `scoot look next`), inert
  while locked (`shell.rs:300-312`), with visible feedback (the bar's look
  name or a toast).

## Evidence to record

Tests: a preview reverting; a preview not reverting a newer apply; the
order wrapping; the bind added to the default keymap table and the docs.

## Not in this ticket

The picker popup (`look-picker`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (2026-10-16, PR #636)

`crates/scootctl/src/look/cycle.rs` (new): catalog-order `cycle_target`
(wrap both ends, unknown/missing choice restarts at the near end) and
`random_target` (`/dev/urandom`, time/pid fallback); the preview file
(`look-preview.json`: `revert-to` + `from-generation`, atomic write,
capped no-follow read, grammar-validated); `revert_if_current` (pre-check
under the lock, guarded revert through `apply_with`, guarded file forget);
`unapply` for first-choice previews; `keep_preview`; the detached
`setsid` reverter argv/spawn. `apply_with` gained `preview` (arm under
the lock, rollback when arming fails) and `only_if_generation` (refuse
as superseded under the lock, atomically). CLI: `next`, `prev`,
`random`, `apply [--for SECONDS] [--keep]`, hidden `_preview-revert`.
Default bind `Super+Shift+T` spawns `scoot look next` with default flags
(inert while locked). Docs in the same PR (looks page, keybindings page,
configure example off the new chord).

Evidence: ticket list all covered (preview reverting, newer-apply-wins,
order wrapping both ends, bind in the default table + docs), each first
as a failing shape then green: 13 cycle unit tests, 4 real-daemon
integration tests (8/8 suite), smoke next/--for checks with pixel proofs.
Full matrices on the dev VM in the report.

## Rebase onto main with the look IPC (2026-10-16, PR #636)

The branch predated the look IPC (#635: `LookApply`/`Looks`, protocol
12, one parked swap), the foot leaf (#633, `RENDER_VERSION` 4) and the
installer (#629). The two cycle commits were transplanted onto main;
`Apply`/`ApplyOptions` carry both sides' fields (`from_compositor` plus
`preview`/`only_if_generation`, and the new `--only-if-generation`
guard flag, which the compositor's revert child passes so a choice that
lands after the guard is read refuses as superseded instead of being
undone). Protocol stays 12, render version stays 4.

The #635 review note (a preview revert silently dropped when a swap runs
at timer fire, sticking the preview) is fixed in the revert path this
ticket owns, on both halves: the compositor stashes the revert and runs
it when the running swap finishes or times out (re-checking the
generation; a revert child locked out past its retries re-arms, bounded,
then gives up loudly), and the detached CLI reverter retries a held lock
(bounded, guards re-checked; anything else returns at once).
