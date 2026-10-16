---
title: "Stylix becomes a synthesized stylix look instead of a layer above the look"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Stylix becomes a synthesized stylix look instead of a layer above the look

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**. Settles "do we keep stylix or create look support with
custom looks": both, with Stylix as a source of a look (D5, D14).

## The gap

Stylix sits above the look (`scootbar.nix:7-20`), baked into the user's own
files, so a swap could never change a key Stylix set; the mapping is
duplicated (`home.nix:571-573`, `scootbar.nix:482-487`).

## What to do

Per the design doc's section 5:

- One Nix function serializing `config.lib.stylix` (sixteen slots, fonts,
  `sizes.desktop`/`popups`/`applications`/`terminal`, `opacity.popups` and
  `terminal`, cursor, image) and a derivation running `scoot-look
  import-base16` on it: the `stylix` look, with
  `programs.scoot.stylix.extends` (default `"ginger-night"` with the desktop
  profile on, none otherwise).
- The declared default `"stylix"` at `mkDefault` (1000), above the
  profile's `mkOverride 1200` and below the user.
- Delete the per-module Stylix leaf mappings; no app module reads Stylix.
  `programs.scoot.stylix.enable` now means "synthesize the `stylix` look";
  `programs.scootbar.stylix.enable` becomes the **bar-only path** (the bar
  module renders the `stylix` look's bar fragment and sets `[look] layer`
  when there is no catalog; an evaluation warning when there is one).
- D14: never render a fragment for a path an upstream module owns
  (`upstreamOwns`, `theme-home.nix:55,75-90`); the apply reply names those
  apps "owned by Stylix's target".
- The changelog states the visible changes of section 5.5 for profile users
  (GTK/Qt, mako, fuzzel and pickers, swaylock, wob and fcitx5 move from
  ginger-night's palette to Stylix's; compositor, bar, wallpaper, terminal
  files and night temperature unchanged) and the one behavior flip for
  explicit-look users.
- Docs in the same PR: `theming.md` 38-44 and 158-189, and the bar's
  theming page.

## Evidence to record

Each Stylix read beyond today's stub checked by hand against a named Stylix
rev, the stub in `nix/tests.nix:754-807` extended, the rev recorded in a
header like `nix/scootbar-tests.nix:9-15`; a test that the `stylix` look's
compositor and bar colors equal today's leaves; a test that Stylix + the
profile evaluates (no priority clash); `scripts/scootbar-stylix-test.sh`
green, including the bar-only path on sway.

## Not in this ticket

Stylix's own targets for apps scoot does not theme.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
