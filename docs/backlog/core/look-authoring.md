---
title: "Authoring: look new, watch and preview capture"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Authoring: look new, watch and preview capture

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**.

## The gap

Making a look today means editing `desktop.nix` and mirroring pins in
`nix/tests.nix` and two docs tables (`site/.../scoot/theming.md:191-210`).

## What to do

- `scoot look new NAME --from BASE`: a source look under `user-looks/` with
  `extends = "BASE"`, BASE a shipped, declared or user-authored source look
  (never a rendered one, never an installed one). A look made with `new` is
  user-authored, so it can be the BASE of the next `new`.
- `scoot look watch DIR`: validate and apply on save, debounced on
  `close_write`/`moved_to`, under the lock, rendering into one reused cache
  slot; an invalid save is refused loudly and the last good look stays.
- `scoot look preview --capture`: a `preview.png` from a private headless
  session.
- Docs: a "make a look" page replacing `theming.md:191-219`.

## Evidence to record

A watch session recorded (three edits, one invalid, the screen following);
the cache not growing per save.

## Not in this ticket

`install`, `export`, `save` (`look-install`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
