---
title: "Nix: the programs.scoot.looks catalog, look as the declared default, the switch rule"
status: "open"
area: "packaging"
priority: "high"
blocked: "look-apply-cli"
---

# Nix: the programs.scoot.looks catalog, look as the declared default, the switch rule

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**: the Nix path is how the maintainer runs the desktop.

## The gap

`programs.scoot.desktop.look` is an enum (`desktop.nix:195-204`) whose
values are merged into `config.toml` and `bar.toml` at module priorities
(`home.nix:532-539`, `scootbar.nix:374-378`), so a swap can never change a
key the look set, and a `home-manager switch` would have no rule for a
choice made at runtime.

## What to do

The design doc's §6:

- `programs.scoot.looks` (names, paths, derivations) and
  `programs.scoot.look` (the declared default, or null), moved out of
  `desktop` per Decision D2. Breaking, shipped clean (`feat(nix)!`), with
  the maintainer's own config updated in step.
- Each look rendered by a derivation running `scoot look build` (validated
  at build time, no import-from-derivation); the catalog linked under
  `~/.local/share/scoot/looks/`; the declared default in a store file.
- Remove the eval-time merges of look values into `programs.scoot.settings`
  and `programs.scootbar.settings`: the runtime layer replaces them.
- An activation step `scoot look sync` applying the switch rule of §6.3
  (no state → default; the declared default changed → it wins; the live
  look gone from the catalog → default, said loudly; otherwise keep the
  live look through the new catalog) and reloading a running session.
- A flake output `lookPacks.<name>` for look repos.
- Retire the Nix registry once every consumer reads look directories.
- Docs in the same PR: `site/.../scoot/theming.md` and
  `site/.../desktop/index.md` (pick a look, the switch rule, rollback).

## Evidence to record

Module tests for each corner of §6.3 (including `look = null` and a
rollback), a VM run of `home-manager switch` with a live choice kept and
with a changed default, and the closure size of the catalog.

## Not in this ticket

Stylix ([`look-stylix-source`](look-stylix-source.md)), the greeter
([`look-greeter`](look-greeter.md)), third-party leaves.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
