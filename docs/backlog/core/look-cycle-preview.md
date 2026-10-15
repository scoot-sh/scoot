---
title: "scoot look next, prev, random and timed previews"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
