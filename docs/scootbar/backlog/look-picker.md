---
title: "A looks picker: a bar popup module and a launcher list"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "look-apply-cli"
---

# A looks picker: a bar popup module and a launcher list

Filed 2026-10-12, child of [`live-looks`](../../backlog/core/live-looks.md).
Serves **daily-drive**.

## The gap

A button can already run `scoot look next` (`on-click = { exec = ["scoot",
"look", "next"] }`, `crates/scootbar/src/config/bindings.rs:6-18,123-140`),
but `[button.*]` cannot open a popup (`config/bindings.rs:86-97`): popups
are built-in modules implementing `Module::popup` (`modules/mod.rs:206-232`),
as `power` does (`modules/power/mod.rs:76-77`).

## What to do

- A built-in `looks` module: the active look's name (the feedback the cycle
  bind needs), and a popup listing the catalog (paged past the 16-widget cap,
  `site/.../scootbar/modules.md:354`), a click running `scoot look apply`. A
  swap reload closes the popup (`daemon/surfaces.rs:260-266`): the expected
  end of a pick.
- A profile script for scootlaunch's dmenu mode (or fuzzel's until then).
- Docs in the same PR.

## Evidence to record

Snapshot tests, a live pick on the dev VM, the idle cost unchanged with the
module placed.

## Not in this ticket

The keybinding (`look-cycle-preview`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
