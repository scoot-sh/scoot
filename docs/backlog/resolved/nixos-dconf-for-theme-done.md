---
title: "The desktop look writes dconf, but the NixOS module never enables programs.dconf"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# The desktop look writes dconf, but the NixOS module never enables programs.dconf

Filed 2026-10-06 from the scoot-iso review (riso2, non-blocking N7). Serves
**daily-drive**: this breaks every NixOS + home-manager user who picks a
look.

## The gap

Since #474 the desktop look writes
`dconf.settings."org/gnome/desktop/interface".color-scheme` from the
home-manager half (`nix/modules/theme-home.nix`). On NixOS, home-manager's
dconf module needs the system's `programs.dconf.enable` (home-manager's own
docs say so). Nothing under `nix/` sets it, which `git grep programs.dconf
nix/` confirms.

The scoot-iso installed system hit this: home-manager's activation did not
land until scoot-iso commit `770d10b` enabled system dconf itself. So any
NixOS user of the desktop profile with a look gets a failed or partial
`home-manager-<user>.service`.

## What to do

- In the NixOS module (`nix/modules/nixos.nix`), set
  `programs.dconf.enable = lib.mkDefault true` whenever the desktop profile
  can write dconf. That is the theme's GTK target, or simply the profile
  being on: say which.
- Pin it in `nix/tests.nix`. A real-NixOS eval (the `osReal*` pattern)
  with the profile and a look must have `programs.dconf.enable == true`,
  and a user's own `false` must still win.
- Docs: one line on the desktop page saying the profile turns dconf on,
  and why.
- Once this lands, scoot-iso can drop its own enable.

## Resolution

Resolved 2026-10-06 in PR #483.

- **Gate: the profile (`desktop.enable`), not the look or the GTK
  target.** `nix/modules/nixos.nix` sets
  `programs.dconf.enable = lib.mkDefault true` in its own
  `lib.mkIf cfg.desktop.enable` element. The dconf write happens on the
  home-manager side (`theme-home.nix`: `themeOn && gtkThemed &&
  toolsReady && !gtkColorSchemeOwned`), which reads the home
  configuration's own `desktop.look` and `theme.targets` -- separate
  copies the NixOS evaluation cannot see. A look set only in the home
  config, or `targets.gtk` off on one side only, would be missed by a
  NixOS-side look/target gate; and when upstream's `gtk` module owns
  the leaf it still writes dconf. Cost is nothing started at boot
  (D-Bus activated), the `dconf` tool and a GIO module path -- the same
  `mkDefault` nixpkgs' `programs/wayland/wayland-session.nix` sets for
  sway, niri, Hyprland and the rest.
- **Pins** (`nix/tests.nix`, real NixOS through `evalRealNixosWith`):
  `osRealDconf` (profile + `moonrise`: on, dconf on the bus),
  `osRealDconfOff` (a plain user `false` wins), `osRealNoProfile`
  (scoot on, profile off: untouched). `nixosStubs` declares the bool so
  the stub evaluations keep evaluating.
- **Docs:** the desktop page's App theme section says the profile turns
  dconf on and why; the Side/Owns table lists system dconf.

Evidence: `nix build .#checks.aarch64-darwin.scoot-modules` (Mac),
`nix build .#checks.aarch64-linux.scoot-modules` (Asahi M2),
`nix build .#docs-site`, repo-wide `nix fmt --check`, `scripts/backlog
check` (only the 3 known pre-existing problems). Failing-first on the
M2: deleting the module line fails `osRealDconf`; `mkForce` instead of
`mkDefault` fails `osRealDconfOff`; gating on `cfg.enable` fails
`osRealNoProfile`.

scoot-iso can now drop its own `programs.dconf.enable` (its commit
`770d10b`).
