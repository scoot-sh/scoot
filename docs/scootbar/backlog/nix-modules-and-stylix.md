---
title: "Nix modules (NixOS and home-manager) with Stylix defaults, and a restart unit"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "nix-package, config-cli-and-reload"
---

# Nix modules with Stylix defaults

Filed 2026-09-29. Serves **daily-drive**.

- The package itself is [nix-package](nix-package.md), already shipped by now;
  this entry adds the modules around it, plus a feature-selectable variant so
  `--features clock,workspaces` is one override.
- A systemd user unit (and its home-manager `Restart=`) so a crashed bar comes
  back, since scoot does not supervise clients
  ([robustness-and-limits](robustness-and-limits.md)).
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
