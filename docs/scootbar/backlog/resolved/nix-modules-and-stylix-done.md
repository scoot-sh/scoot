---
title: "Nix modules (NixOS and home-manager) with Stylix defaults, and a restart unit"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-10-01"
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

## Resolution (2026-10-01, PR #370)

First landing (2026-09-30): the home-manager and NixOS modules
(`nix/modules/scootbar*.nix`), the `features` override, Stylix defaults with the
pinned precedence, the user unit, docs and `checks.<system>.scootbar-modules`.

Finished 2026-10-01 (PR #370; evidence keyed to `79013859` on the Asahi box,
aarch64-linux, systemd 261.2, Nix 2.34.8; raw logs in `~/fx/nixrest-final-*.log`
there):

- **A home-manager unit under a real user manager**: `scripts/scootbar-unit-test.sh
  --home` builds a real home-manager (`efa3ccb`) generation of `homeModules.scootbar`,
  runs its `activate` for real into a scratch home, loads the unit it linked and runs
  S0 to S9: `RESULT: PASS 36  FAIL 0`. `--nixos` (the NixOS-generated unit, now
  taken from the real generated user-unit directory): `PASS 33  FAIL 0`.
- **`graphical-session.target`**: found the documented `systemctl --user start
  graphical-session.target` is refused (`RefuseManualStart=yes`); the docs now show a
  session target that `BindsTo=` it. S8: started with the environment imported first,
  the bar came up 0.6 s later on its first try, after the target, and stopped with it.
- **Restart triggers**: S9 with sd-switch 0.6.4: an unchanged unit is `No action
  ... RestartEq` (same `InvocationID` and pid); a changed config is `Stop/Start`
  and the new bar draws the new height. NixOS: read from the pinned
  `switch-to-configuration-ng`, not run.
- **Real Stylix**: `scripts/scootbar-stylix-test.sh`, Stylix `fb28acd` and
  home-manager from a real image: `PASS 8  FAIL 0`; tokens are base16 base00/05/0A/03/08,
  `scootbar daemon --check` accepts the file, a user value wins per key, the bar's
  background pixel is `base00` on headless scoot.
- **`scootbar daemon --check`**, used by the module check, which no longer fakes a
  missing compositor. Built on aarch64-linux (`nix build .#checks.aarch64-linux.scootbar-modules`).
- **The `session.command`/autostart pairing** is written (docs/nix.md).
- **x86_64**: no x86 user manager was run (decided: it would add no kernel and
  systemd behavior does not vary by CPU). x86_64-linux evaluates (`nix eval` of the
  package, check, NixOS unit and home-manager generation drvPaths) and CI's `scootbar`
  job now builds `checks.x86_64-linux.scootbar-modules` on an x86_64 runner, as does the
  Linux job's `nix flake check`.

Not covered, by design or for want of a machine: a NixOS switch (not run:
`nixos-rebuild` is not for a shared machine), a lingering user manager outliving
the session, and a Stylix font family that resolves to a variable font (the
fallback is pinned in `nix/scootbar-tests.nix`, not run against a real package).

## Done when

`programs.scootbar.enable = true` gives a themed, running bar under
home-manager with and without Stylix, and the docs list the options.
