---
title: "The desktop look writes dconf, but the NixOS module never enables programs.dconf"
status: "open"
area: "packaging"
priority: "high"
blocked: null
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
