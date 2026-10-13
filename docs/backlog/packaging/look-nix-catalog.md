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

`programs.scoot.desktop.look` is an enum (`desktop.nix:195-204`) merged into
`config.toml` and `bar.toml` at module priorities (`home.nix:532-539`,
`scootbar.nix:374-378`), so a swap can never change a key the look set, and a
`home-manager switch` has no rule for a choice made at runtime.

## What to do

Per the design doc's sections 3.1, 5.2 and 6:

- `programs.scoot.looks` (names and source paths, read with
  `builtins.fromTOML`; no derivations, no IFD) and `programs.scoot.look`
  (D2), with the profile's default moved from `mkDefault` to
  `mkOverride 1200 "ginger-night"` (`home.nix:503-505`,
  `nixos.nix:2235-2237`), leaving priority 1000 for the Stylix source and
  1500 to the option's `default = null`. Breaking (`feat(nix)!`), the
  maintainer's config updated in step.
- Each look rendered by a derivation running the `scoot-look` binary with
  its `[fonts.nix]` packages as inputs (D13); the catalog linked as
  `~/.local/share/scoot/looks/<name>` (sources) and
  `~/.local/share/scoot/rendered/<name>` (renders); the declared default in
  `~/.config/scoot/look-default`.
- Remove the eval-time merges of look values into `programs.scoot.settings`
  and `programs.scootbar.settings`.
- The activation step `scoot look sync` (the rule of section 6.3), ordered
  after `linkGeneration` and `dconfSettings`, and home-manager no longer
  writing `color-scheme` from an eval-time look (D11), except a key the user
  set in `dconf.settings`.
- The NixOS-module-only path: the catalog under
  `/run/current-system/sw/share/scoot/`, `/etc/scoot/look-default`, and
  `scoot look sync` run by `resources/scoot-session` at login.
- Docs in the same PR: `site/.../scoot/theming.md` and
  `site/.../desktop/index.md`.

## Evidence to record

Module tests for each corner of section 6.3 (`look = null` and a rollback
included), for the priority table (Stylix + profile evaluates to `stylix`;
an explicit look wins; the profile alone gives ginger-night), a VM run of
`home-manager switch` keeping a live choice and following a changed default,
and the catalog's closure size.

## Not in this ticket

Stylix, the greeter, third-party leaves, retiring the registry,
`lookPacks` (later, `look-share`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
