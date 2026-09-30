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

**Run under a real user manager, 2026-09-30** (the Asahi M2, NixOS aarch64,
systemd 261; `scripts/scootbar-unit-test.sh`, 17 PASS, 0 FAIL): the unit from
`nixosModules.scootbar` (a real NixOS evaluation, not a hand-written unit) and
the Nix-built `scootbar`, loaded into the user manager's runtime directory
against a live headless scoot. Started with no compositor it kept retrying
(6 restarts in 14 s, never failed) and came up on the first retry after
`WAYLAND_DISPLAY` reached the manager; a SIGKILL was followed by a restart and a
redrawn bar; eight SIGKILLs in a row never left it failed; a SIGTERM,
`scootbar msg kill` and `systemctl --user stop` each stayed stopped. The
control the docs had not had: with systemd's *default* start limit the same
retry also kept going (9 restarts in 22 s, the script's S7), so `StartLimitIntervalSec=0` is
insurance against a shorter `RestartSec`, not what keeps the unit alive; the
module comments and nix.md said otherwise and are corrected. A first run failed
S5 on the script's own race (a `Type=simple` unit is "active" before the bar has
its control socket), not a product fault.

Left, so the entry stays open: the home-manager-generated unit (the run above is
the NixOS module's; the shapes are shared, not run) and a themed run with real
Stylix and home-manager; `graphical-session.target` started by a session script
(the unit was started directly), and `X-Restart-Triggers` on a real switch;
x86_64 (the run was aarch64); a build of the check on aarch64-linux (only
evaluated); the `session.command`/autostart pairing sentence in the docs; and a
`scootbar --check` flag, which would let the check validate a file without
faking a missing compositor.

## Done when

`programs.scootbar.enable = true` gives a themed, running bar under
home-manager with and without Stylix, and the docs list the options.
