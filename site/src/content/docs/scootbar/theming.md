---
title: scootbar theming
description: "Theme the bar from a look or Stylix — colors, fonts, and what wins per key."
---

The bar follows the desktop: the profile's `look` writes its `colors`,
and Stylix writes colors plus font when present. A value you set in
`settings` always wins — per key, never wholesale.

## From a look

`look` renders the example palette into the bar config (each leaf
yielding to a user value, and to Stylix where present):

| Look | Bar colors |
|---|---|
| `ginger-night` (default) | near-black, cream and ginger |
| `music-desk` | paper, ink and blue |
| `radial-burst` | plum, yellow and blue |
| `moonrise` | navy, cream and amber |
| `vinyl-sunset` | espresso, cream and orange |

Opt out per target while the rest follows the look, under the one
`programs.scoot.desktop.theme.targets.<name>.enable` namespace every
desktop piece uses (as with the [lock screen](../desktop/index.md#idle-and-lock)).

## From Stylix

When `config.lib.stylix` exists and `stylix.enable` is on, `settings`
gets defaults from it: the six `colors` tokens from the base16 palette
(`background` base00, `foreground` base05, `accent` base0D, `hover`
base0D like the compositor's focused ring, `dim` base03, `urgent`
base08), `bar.font` from `stylix.fonts.sansSerif` and `bar.font-size`
from `stylix.fonts.sizes.desktop` (points, converted at 4/3, clamped
1–256). The font is a **file path**: the bar has no fontconfig, so a
build step finds the regular face in the font package (`DejaVuSans.ttf`
for "DejaVu Sans", `Family-Regular.ttf` for Noto or a Nerd Font).
Variable fonts, `.ttc` collections and odd namings are not resolved —
the bar then gets the plain default font rather than a failed build.
`programs.scootbar.stylix.enable = false` turns the defaults off with
Stylix present.

Precedence, highest first, per key: your `settings` value, then
Stylix's, then the module's plain default (`bar.font` as DejaVu Sans —
also what an unresolved Stylix font becomes). So
`settings.colors.background = "#123456"` replaces that one token and
keeps the other five. The bar's own unthemed defaults stay Catppuccin
Mocha (`background #1e1e2e`, `accent #f9e2af`); only the Stylix default
moved accent to base0D blue, matching the compositor's Stylix ring.

The six tokens, and how icons follow them (path icons tint from the
theme; image icons keep their own colors), are in
[Colors](./configure.md#colors) and [Icons](./configure.md#icons).
