---
title: "Later: install looks from git and flake lookPacks"
status: "open"
area: "core"
priority: "low"
blocked: "look-install"
milestone: "later"
---

# Later: install looks from git and flake lookPacks

Filed 2026-10-12, child of [`live-looks`](live-looks.md). **Later** (D12):
filed so the design has a home, not scheduled. Serves **daily-drive**.

## What to do (when scheduled)

- `scoot look install GIT-URL`: `git clone --depth 1
  --no-recurse-submodules -- URL`, `https://` and `ssh://` only, then the
  `install PATH` checks.
- A flake output `lookPacks.<name>` (a directory of looks usable as a flake
  input in `programs.scoot.looks`).

## Not in this ticket

A central registry (D6: none).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
