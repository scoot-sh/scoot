---
title: "Workspace wallpapers in scoot's [wallpaper] section"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
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

## Resolution (2026-10-08, PR #542)

Both shapes the ticket offered, since the daemon models `(output,
workspace)` with output `None` for every output: `workspace."2"` maps a
workspace on every output, `output."DP-1".workspace."2"` on one output
alone (winning where both name it). Each table carries the image/color
keys plus transition keys and stands alone; an empty table clears that
mapping (`Clear`, as `clear --workspace`); absent tables leave live
mappings alone (as an absent output table does). `Section::record` feeds
`Record.workspaces`; `daemon::config` put/recheck keep the
`set --workspace`-since-wins precedence at the old generation
(`Choices::fill_workspace`); the canonical encoding and fingerprint gain
the tables; mapped images preload on apply through the restore path, and
the manager binds/releases around apply. scoot's `[wallpaper]` accepts,
resolves and forwards the same tables. Docs in the same PR (site table +
workspaces page, `docs/scootbg/README`, versioning note, default-config
comments). Tests, each revert-run-restore proven: four scootbg section
tests, one choices test, two scoot section tests. Verification: fmt,
clippy (scootbg + scoot), nextest 636 passed with only load-induced timer
flakes (rotation set fails identically on main; transition/color pass on
retry), scoot wallpaper 62 passed, docs-site build, deny, ratchet
(file +3.6%, .text +1.5%; idle 0 wakeups both sides, RSS +48 kB inside the
5% noise rule: tie; nothing waived).
