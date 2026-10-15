---
title: "install.sh --look installs the catalog and applies a look instead of copying whole configs"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# install.sh --look installs the catalog and applies a look instead of copying whole configs

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive** for non-Nix users.

## The gap

`install.sh --look` writes the example's whole `scoot.toml` and `bar.toml`
only where no file exists (`site/public/install.sh:411-422,424-462`), so it
cannot change the look of an existing install, and the files it wrote now
shadow every look.

## What to do

- Install the shipped looks under the prefix (recorded in the manifest so
  `--uninstall` removes them) and run `scoot look apply NAME`.
- Stop writing whole config files for `--look`.
- Migrate what earlier runs wrote: a `config.toml` or `bar.toml` still
  exactly what an earlier `install.sh --look` produced (after its path
  rewrites, `site/public/install.sh:428-459`) has its look keys removed so
  the layer shows through; any other file is left alone and its shadowing
  keys named.
- Docs in the same PR: the install page's `--look` row.

## Evidence to record

A clean box and one with existing configs (both end on the chosen look; an
edited file byte-identical), and `--uninstall` leaving no look files.

## Not in this ticket

Third-party leaves on non-Nix (the leaves tickets).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
