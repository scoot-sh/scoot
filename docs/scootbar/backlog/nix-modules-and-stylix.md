---
title: "Nix modules (NixOS and home-manager) with Stylix defaults, and a restart unit"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M3"
---

# Nix modules with Stylix defaults

Filed 2026-09-29. Serves **daily-drive**.

- The package itself is [nix-package](resolved/nix-package-done.md), already shipped by now;
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
  (a file path: scootbar has no fontconfig and only a short fixed directory list); otherwise fall back to
  the plain defaults. Explicit user values always win.
- **Precedence, written down and tested**: an explicit user value beats Stylix,
  which beats the module's plain defaults. The commonest Nix complaint about other
  bars is a Stylix default that silently overrides the user's own styling (Waybar
  #3748, and the documented `mkAfter` workaround); scootbar has tokens, not CSS, so
  the rule is simple, but it must hold and be pinned by an eval test.
- **Freeform `settings`** rendered to the config file, so a new option never needs
  a module change first; typed options only for the few that need them.
- Fonts and icon files the bar needs must be reachable from its environment
  (a path, not an ambient font search), and the user unit orders after the
  graphical session and before tray-hosting apps where it can.
- A native `stylix.targets.scootbar` is an upstream change and stays the
  maintainer's decision (`CLAUDE.md`); nothing here opens one.
- Checks: the module evaluates on both systems, the rendered file parses with
  the real binary, and CI's flake checks cover it.

## Status (2026-09-30)

Landed: the home-manager and NixOS modules (`nix/modules/scootbar*.nix`), the
`features` override, Stylix defaults with the pinned precedence, the user
unit, docs ([nix.md](../../nix.md#the-modules-programsscootbar)) and
`checks.<system>.scootbar-modules` (evaluation with and without a Stylix
stand-in, the real binary over each rendered file; no `--check` flag exists,
so it runs `daemon --config FILE` with no compositor and expects the
connection refusal). Also evaluated by hand against real Stylix `fb28acd`
with real NixOS and home-manager.

Left, so the entry stays open: run the unit under a real user manager against
a running scoot session (`graphical-session.target` started by a session
script, restart after `kill -SEGV`), on both architectures; a build of the
check on aarch64-linux (only evaluated); the `session.command`/autostart
pairing sentence in the docs; and a `scootbar --check` flag, which would let
the check validate a file without faking a missing compositor.

## Done when

`programs.scootbar.enable = true` gives a themed, running bar under
home-manager with and without Stylix, and the docs list the options.
