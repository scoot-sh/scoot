---
title: "A Stylix target for the compositor: window borders, background, cursor, wallpaper"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
milestone: "M6"
---

# A Stylix target for the compositor: window borders, background, cursor, wallpaper

Filed 2026-10-03. Serves **daily-drive**: the maintainer themes their whole
scoot desktop with Stylix (light polarity, from their wallpaper) and today
sets the workspace pill and the bar's highlights to the scheme's blue by
hand, while the compositor's window rings, background, cursor and wallpaper
get nothing from the scheme at all.

## The gap

`nix/modules/scootbar.nix` has Stylix defaults
(`programs.scootbar.stylix.enable`, six `colors` tokens from
`config.lib.stylix.colors.withHashtag`, font and size), pinned by
`nix/scootbar-tests.nix`, but the compositor's modules
(`nix/modules/home.nix`, `nix/modules/nixos.nix`) have none. Scoot's config
already has the keys a target would set (`docs/configuration.md`
`[appearance]`: `focus_ring_active_color`, `focus_ring_inactive_color`,
`background_color`, `cursor_color`, `cursor_theme`, `cursor_size`; and the
`[wallpaper]` table, `image = ...`, drawn by scootbg) — they just keep their
built-in defaults under a Stylix setup.

## What to do

1. `programs.scoot.stylix.enable` (and the NixOS equivalent if the NixOS
   module renders the config too), built exactly the way `scootbar.nix`
   does it: same detection (`config.lib ? stylix && config.stylix.enable`),
   on by default when Stylix is in use, every value at `lib.mkDefault` so a
   value the user wrote wins, no Stylix input on this flake, nothing
   changes without Stylix.
2. The mapping, confirmed against Stylix's own window-manager targets
   (sway, hyprland, niri, river) at the rev `scootbar-tests.nix` names:
   focused ring, unfocused ring, background, cursor (`stylix.cursor`
   name/size/package), wallpaper (`stylix.image`, `stylix.imageScalingMode`
   against what scootbg supports).
3. The bar accent: change scootbar's Stylix `accent`/`hover` default from
   `base0A` to whatever the compositor's focused ring uses, so the bar's
   highlights and the window ring match out of the box.
4. Tests: extend the nix eval tests the way `scootbar-tests.nix` stubs
   Stylix (precedence user > Stylix > built-in default; nothing without
   Stylix; evaluates with no Stylix option defined).
5. Docs: `docs/nix.md` (the new option, what it sets, the mapping table
   with base16 names) and wherever `docs/configuration.md` mentions Nix
   or Stylix.

## Not in this ticket

Scheme generation, polarity handling beyond what the base16 slots already
encode, GTK/Qt app theming (Stylix's own targets do that), and any change
to scoot's runtime.
