---
title: "Workspace wallpapers in scoot's [wallpaper] section"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# Workspace wallpapers in scoot's [wallpaper] section

Filed 2026-10-08, split out of `resolved/per-workspace-done.md` (one PR holds the
daemon + CLI slice; this is the config half). Serves **daily-drive**:
workspace wallpapers set in one place with the rest of the desktop,
re-applied on reload like every other `[wallpaper]` key.

## The gap

`scootbg set --workspace` (resolved/per-workspace-done.md) manages workspace
wallpapers through the CLI and the state file only. scoot's
`[wallpaper]` section (`crates/scootbg/src/section.rs`) has no keys for
them: `apply-config` leaves live mappings alone (documented, and
covered by the daemon's restore path), so a config cannot set, change
or remove one, and a reload cannot repair a mapping deleted from under
it.

## What to do

Extend the section schema with per-workspace tables, mirroring the
per-output ones: e.g. `output."DP-1".workspace."2"` (or a
`workspace."2"` table beside `output`), each the image/color keys plus
transition keys, standing alone like every other table. Wire through
`Section::record` (into `Record.workspaces`, which already exists),
`config::put`/`recheck` precedence (a `set --workspace` made since wins
until the section changes, as for base choices), the fingerprint
(canonical encoding gains the tables), and the site `[wallpaper]`
table. Preload on apply like the daemon's `configure` hook does.

## Not in this ticket

Rotation (`config-and-rotation.md`) interplay; animated per-workspace
images (`animated-images.md` covers the format, this covers the mapping).
