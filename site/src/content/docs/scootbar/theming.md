---
title: scootbar theming
description: "Theme the bar from a look or Stylix — colors, fonts, and what wins per key."
---

The bar reads a look layer under its own file: `<state>/look/bar.toml`
merged per key, with the file winning every key it sets. A value set in
`settings` (Nix) or `bar.toml` always wins — per key, never wholesale.

## From a look

The layer carries the six `[colors]` tokens, the `[bar]` style keys
(`radius`, `popup-radius`, `opacity`, `font-size`, `padding`, `spacing`,
`separator`), the workspaces pill keys and the per-module glyph icons.
It never carries geometry, placement, module lists, commands or
actions. `hover` follows a custom `accent` when unset: a file that sets
only `accent` derives its hover from it; a file that sets only `hover`
keeps the look's `accent`.

A bar without scoot has no state link. It pins a rendered fragment
instead:

```toml
[look]
layer = "/run/current-system/sw/share/scoot/rendered/moonrise/bar.toml"
```

Without Nix, render the fragment with `scoot-look build` and point
`[look] layer` at it (or place it at the state link). See
[Look layer](./configure.md#look-layer) for the full precedence, the
fixed layer, and the reload scopes.

The shipped looks, and what each gives the bar:

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

Those values land in the file rung, so each shadows the look layer per
key, exactly like a value written by hand. To let a look show through,
unset the key (or turn the target off). Precedence, highest first, per
key: daemon flags, the file (hand-written, Stylix-derived or
look-derived alike), the look layer, the module's plain default.

The six tokens, and how icons follow them (path icons tint from the
theme; image icons keep their own colors), are in
[Colors](./configure.md#colors) and [Icons](./modules.md#icons).
