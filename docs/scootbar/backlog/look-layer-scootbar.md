---
title: "scootbar reads a look layer under bar.toml, with a look-scoped reload"
status: "open"
area: "scootbar"
priority: "high"
blocked: "look-format"
---

# scootbar reads a look layer under bar.toml, with a look-scoped reload

Filed 2026-10-12, child of [`live-looks`](../../backlog/core/live-looks.md).
Serves **daily-drive**.

## The gap

scootbar reads one `bar.toml` (`crates/scootbar/src/config.rs:244-286`).
Its reload is whole (`daemon/respond.rs:177-282`) but re-reads the user file
too, so a swap would apply pending edits. Some keys exist only with a cargo
feature (`popup-radius`, `config.rs:396-401`), and `deny_unknown_fields`
refuses them on a trimmed build. SIGHUP kills the bar (`daemon/mod.rs:27-35`),
so the control socket is the trigger.

## What to do

- Read `<state>/look/bar.toml` (the state resolution of the design doc's
  section 4.1, `SCOOT_LOOK_STATE` from scoot), merged under the user's file
  on raw keys with `accent` + `hover` as one unit (the bar derives `hover =
  accent`, `config.rs:1692`). The flags rung (`--background`,
  `--foreground`) stays above both (`respond.rs:184-189`).
- The allowlist, shared with the design doc via `scoot-look`: `[colors]`;
  `[bar]` `radius`, `popup-radius`, `opacity`, `font`, `fallback-fonts`,
  `font-size`, `padding`, `spacing`, `separator`; the per-module visual keys
  (`workspaces` `pill-shape`, `pill-inset`, `disc`, `inactive-color`, the
  module glyph keys under the glyph grammar). Explicitly out: `edge`,
  `height`, `margin`, `layer`, `exclusive`, `tooltip-delay`, module lists,
  `exec`/`button`/`push` tables, `on-*`. A look therefore never touches the
  `Bar` keys (`bar.rs:185-197`) and never recreates the layer surface.
- A key whose feature is not compiled in is dropped from the *layer* with a
  note (the user's own file keeps `deny_unknown_fields`).
- `{"type":"reload","scope":"look"}`: the user file as parsed at the last
  successful load, the new layer under it.
- A fixed layer for the bar-only path: a `[look] layer = PATH` key in
  `bar.toml`, read like the state link, for users who run the bar without
  scoot (the design doc's section 5.5).
- Docs in the same PR: `site/.../scootbar/configure.md` and `theming.md`.

## Evidence to record

Tests for precedence, the unit, refusals, the feature drop, the scoped
reload not applying a pending module-list edit, and the fixed layer.

## Not in this ticket

Keeping modules across a style-only reload
([`scootbar-style-reload`](scootbar-style-reload.md)), the picker.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
