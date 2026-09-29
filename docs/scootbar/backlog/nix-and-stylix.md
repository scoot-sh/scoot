---
title: "Nix modules (NixOS and home-manager) with Stylix defaults"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "config-cli-and-reload"
---

# Nix modules with Stylix defaults

Filed 2026-09-29. Serves **daily-drive**.

- `packages.scootbar`, following how `packages.scoot` and scootbg are packaged
  (`nix-src-fileset-done.md`, `scoot-package-ships-scootctl-done.md`); a
  feature-selectable variant so `--features clock,workspaces` is one override.
- NixOS and home-manager modules whose `settings` attrset renders the config
  file, as scoot's home-manager module does; `enable` adds the package and
  an autostart entry (or `session.command` pairing, per `nixos-session-command-done.md`).
- **Stylix without depending on it**: when `config.lib.stylix` exists, default
  the semantic color tokens from its base16 colors and the font from its font
  (a file path, since scootbar does no font discovery); otherwise fall back to
  the plain defaults. Explicit user values always win.
- A native `stylix.targets.scootbar` is an upstream change and stays the
  maintainer's decision (`CLAUDE.md`); nothing here opens one.
- Checks: the module evaluates on both systems, the rendered file parses with
  the real binary, and CI's flake checks cover it.

## Done when

`programs.scootbar.enable = true` gives a themed, running bar under
home-manager with and without Stylix, and the docs list the options.
