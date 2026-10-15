---
title: "install.sh --look installs the catalog and applies a look instead of copying whole configs"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
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

## Resolution (2026-10-15)

PR: https://github.com/scoot-sh/scoot/pull/629 (open, not merged).

`--look NAME` installs all five shipped looks as plain files under
`<prefix>/share/scoot/looks` (each `look.toml` plus its wallpaper image;
vinyl-sunset is a flat color and ships no image), records them in the
install manifest, migrates old whole-config files, and runs
`<bin>/scoot look apply NAME`. It no longer writes `config.toml`,
`bar.toml`, helper scripts or wallpapers into the user config. A
`config.toml`/`bar.toml` still byte-identical to an earlier run's output
(with that run's wallpaper and script-path rewrites) has its look keys
stripped (whole `[appearance]`/`[wallpaper]`/`[colors]` sections, the
look's `[bar]` style keys, the look's `[apps.scootbar]` overrides;
layout kept) so the layer shows through. Any other file is untouched and
the apply reply names its shadowing keys. A refused apply never stops
the install: binaries, catalog, units and the library report land first,
and the run exits 1 at the end pointing at a re-run with `--look`. Under
`sudo` the apply runs as the invoking user (sudo/doas `-u`, else su,
doas, runuser or setpriv for root), with standard XDG defaults under
their home and their session's runtime dir, so system installs never
leave root-owned state behind.

Evidence: `site/scripts/test-install.sh` (26 checks: catalog placement
and manifest, no whole configs, byte-identical migration for rewrite and
no-rewrite looks, edited files left alone with shadowing named, refused
apply failing last, uninstall removing files and emptied dirs, sudo
dry-run as the invoking user; shellcheck clean; passes under bash and
dash, on macOS and Linux) and a real-binary end to end on the dev VM
against a file:// release (clean install applies moonrise; existing
install switches to ginger-night; real example files migrate with
`shadowed keys: none`; an edited file keeps its bytes with shadowing
named; `--uninstall` leaves no look files; a real `--system` install
applies as the invoking user with dev-owned state, through both the
sudo and su paths, then uninstalls clean). Two behaviors found by that
end to end and fixed in the PR: running the apply as root into a user's
dirs (root-owned state locked them out) and empty (not unset) XDG
variables hiding `~/.local/share/fonts` from fontconfig. A box without
the look's Nerd fonts still refuses at apply, by the apply's documented
contract, with the install complete underneath.
