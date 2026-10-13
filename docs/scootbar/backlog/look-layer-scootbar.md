---
title: "scootbar reads a look layer under bar.toml, and a style-only reload keeps its modules"
status: "open"
area: "scootbar"
priority: "high"
blocked: "look-format"
---

# scootbar reads a look layer under bar.toml, and a style-only reload keeps its modules

Filed 2026-10-12, child of [`live-looks`](../../backlog/core/live-looks.md).
Serves **daily-drive**: the bar is the second most visible piece of a look.

## The gap

scootbar reads one `bar.toml` with no layer (`crates/scootbar/src/config.rs:244-286`).
Its reload is live and whole (`daemon/respond.rs:177-282`), but every reload
restarts every module except an unchanged `exec`
(`modules/mod.rs:615-661`), so a colors-only swap tears down and rebuilds
every D-Bus client, and it closes any open popup (`daemon/surfaces.rs:260-266`).
A geometry change destroys and recreates the layer surface
(`daemon/surfaces.rs:292-309`); whether that blinks has never been measured.
SIGHUP kills the bar (`daemon/mod.rs:27-35`), so the control-socket reload
is the only trigger, which is the one the design keeps.

## What to do

- Read the layer from `$XDG_STATE_HOME/scoot/look/bar.toml`, merged under
  the user's `bar.toml` per key. The allowlist: `[colors]`; `[bar]` `radius`,
  `popup-radius`, `opacity`, `font`, `fallback-fonts`, `font-size`,
  `padding`, `spacing`, `separator`; the per-module visual keys (workspace
  `pill-shape`, `pill-inset`, `disc`, `inactive-color`, ...). Never module
  lists, `exec`/`button`/`push` tables, `on-*` actions, `edge`, `layer` or
  `exclusive`. A forbidden key refuses the layer by name.
- A reload whose only change is style (colors, the style keys, the font)
  keeps every module and its state and repaints; only a layout or module
  table change restarts modules.
- The reply says which layer was refused and why (today a refused reload is
  reported only to the client: `respond.rs:212-222`).
- Docs in the same PR: `site/.../scootbar/configure.md` and `theming.md`.

## Evidence to record

Tests for precedence, refusals and the style-only path keeping module
state (a module's subscription survives). Before/after numbers for a
colors-only reload: wall time, CPU, allocations, D-Bus reconnects (zero
after). A screen capture across a geometry reload to settle the blink
question.

## Not in this ticket

The picker ([`look-picker`](look-picker.md)), the CLI.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
