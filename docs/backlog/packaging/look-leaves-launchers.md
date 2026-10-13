---
title: "Look leaves for swaylock, fuzzel and the pickers"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# Look leaves for swaylock, fuzzel and the pickers

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

swaylock runs with `-C ${swaylockConfig}`, a store path
(`idle-home.nix:83`); fuzzel's flags are inlined into the launcher and picker
scripts (`keys-home.nix:72-79,139-141`, `apps-home.nix:67-73`) and the
capture chooser (`capture-home.nix:41-42,69-76`).

## What to do

- swaylock: `-C <state>/look/swaylock` (next lock).
- fuzzel: `--config <state>/look/fuzzel.ini` instead of inlined flags (next
  launch), the launcher, pickers and capture chooser alike.
- Per-format quoting (section 3.5): fuzzel's `launch-prefix` and `terminal`
  keys are exactly why a font name must never carry a newline.

## Evidence to record

A swap observed at the next lock and the next launcher open; a quoting test.

## Not in this ticket

scootlaunch (its own plan, reading the `launch.toml` fragment).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
