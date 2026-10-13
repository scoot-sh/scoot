---
title: "A looks picker: a bar popup module, a default keybinding, a launcher list"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "look-apply-cli"
---

# A looks picker: a bar popup module, a default keybinding, a launcher list

Filed 2026-10-12, child of [`live-looks`](../../backlog/core/live-looks.md).
Serves **daily-drive**.

## The gap

A button can already run `scoot look next`
(`on-click = { exec = ["scoot", "look", "next"] }`,
`crates/scootbar/src/config/bindings.rs:6-18,123-140`), but a picker needs a
popup, and `[button.*]` cannot open one (`config/bindings.rs:86-97`): popups
are built-in modules implementing `Module::popup`
(`modules/mod.rs:206-232`), as `power` does (`modules/power/mod.rs:76-77`).

## What to do

- A built-in `looks` module: the active look's name or swatch, and a popup
  listing the catalog (paged past the 16-widget cap,
  `site/.../scootbar/modules.md:354`), a click running `scoot look apply`.
  The bar reload a swap triggers closes the popup (`daemon/surfaces.rs:260-266`);
  that is the expected end of a pick, stated in the docs.
- A default keybinding for `scoot look next` (Decision D8; recommended
  `Mod+Shift+T`, checked against the default keymap).
- A profile script for scootlaunch's dmenu mode (or fuzzel's until then):
  `scoot look list` → pick → `scoot look apply`.
- Docs in the same PR: `site/.../scootbar/modules.md`, the keybinding table.

## Evidence to record

The module's snapshot tests, a live pick on the dev VM, and the bar's idle
cost unchanged with the module placed (the resource ratchet).

## Not in this ticket

The apply engine.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
