---
title: "install.sh --look installs the catalog and applies a look instead of copying whole configs"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# install.sh --look installs the catalog and applies a look instead of copying whole configs

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive** for non-Nix users.

## The gap

`install.sh --look` downloads the example's whole `scoot.toml` and
`bar.toml` and writes them only where no file exists
(`site/public/install.sh:411-422,424-462`), so it cannot change the look of
an existing install, and it copies a layout along with the look.

## What to do

- Install the shipped looks into `$XDG_DATA_HOME/scoot/looks/` (recorded in
  the install manifest so `--uninstall` removes them) and run `scoot look
  apply NAME`.
- Stop writing whole config files for `--look`; a user's existing files are
  never touched.
- Migrate what earlier runs wrote: a `config.toml` or `bar.toml` that is
  still exactly what an earlier `install.sh --look` produced (compare with
  what install.sh would write for that look, after its path rewrites at
  `site/public/install.sh:428-459`) has the look's keys removed so the look
  layer can show through; any other file is left alone and its shadowing
  keys are named (`scoot look current` lists them).
- Docs in the same PR: the install page's `--look` row.

## Evidence to record

A run on a clean box and on one with existing configs (both end on the
chosen look; the existing files byte-identical), and `--uninstall` leaving
no look files.

## Not in this ticket

Third-party leaves on non-Nix (they come with
[`look-app-leaves`](look-app-leaves.md)).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
