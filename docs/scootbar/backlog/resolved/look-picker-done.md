---
title: "A looks picker: a bar popup module and a launcher list"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
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

## Resolution (2026-10-15)

- Built-in `looks` module (`crates/scootbar/src/modules/looks/`, Cargo
  feature `looks`, on by default): the bar text is the recorded look's
  name read fresh on every view (`no look` with nothing applied yet), a
  click opens the picker, and the picker lists the catalog in search
  order with the active look selected, paged at 14 rows with `‹ prev`
  and `next ›` rows inside the 16-widget cap. A row stages
  `scoot look apply NAME` for the bar to spawn and closes the picker;
  `apply N` names the picker's Nth shown row (a row past the snapshot is
  refused), `page N` turns the page. No fd, no timer: placing it costs
  no wakeups. The active row carries the module's glyph `icon` before
  its name where one is set, as the power menu's row glyphs do; without
  one the accent selection alone marks it.
- One catalog implementation: `crates/scootctl/src/look/catalog.rs` moved
  to `crates/scoot-look/src/catalog.rs`, used by `scoot look list` and
  the picker alike.
- Launcher list: `resources/scoot-look-pick` (`list` names in catalog
  order, `menu` stdin through `fuzzel --dmenu`, `pick` applies the
  choice; a cancel applies nothing), documented on the looks page for
  Nix and non-Nix use.
- Docs: `site/src/content/docs/scootbar/modules.md` (`## Looks`),
  `site/src/content/docs/looks/index.md` (picker paragraph),
  `scootbar daemon --help` (`Looks` section, gated on the feature).

Evidence: unit tests through the harness (bar text follows the record,
one page with the active selected, paging past the widget cap, rows
stage `scoot look apply` for the snapshot's name, rows and pages past
the snapshot refused, the configured glyph marking the active row);
`crates/scootbar/tests/look_picker.rs` against a
live headless session (an outside apply moves the bar text; opening the
picker then `invoke looks "apply 0"` runs a real apply that swaps the
desktop and moves the bar text); a manual live pick on the dev VM
(center background pixel #2B3648 to #3B4A5C, bar text `no look` to
`pick-b`, `scoot look current` agreeing); idle 0 wakeups in 15 s with
and without the module placed, 7 fds both.
PR: #637.
