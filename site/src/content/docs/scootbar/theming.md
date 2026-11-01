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
`separator`, `shadow`, `fill`), the workspaces pill keys and the per-module glyph icons.
It never carries geometry, placement, module lists, commands or
actions. `hover` follows a custom `accent` when unset: a file that sets
only `accent` derives its hover from it; a file that sets only `hover`
keeps the look's `accent`. `shadow` (`none`, `soft`, `deep` or `glow`)
and `fill` (`solid` or `gradient`) lift the bar and its popups off the
windows behind them; a look that sets neither renders the fragment it
always did. Ginger-night (the default) takes a soft shadow; radial-burst
takes a glow.

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
desktop piece uses (as with the [lock screen](../desktop/idle-lock.md)).

## From Stylix

Without scoot, a bar under Stylix themes from the `stylix` look
instead of eval-time values: nothing Stylix lands in `settings`.
With a scoot catalog in the configuration, the bar follows the
`stylix` look like any look (through `scoot look sync`), and this
path stays off.

- `programs.scootbar.stylix.enable` — Type: bool. Default: `true`.
  The bar-only path: with Stylix present and enabled, and no scoot
  look catalog in this configuration, render the `stylix` look's bar
  fragment at build time (the same `scoot-look` binary the catalog
  uses) and pin it as a fixed `[look] layer` in the rendered
  `bar.toml`. A value you set in `settings` still wins per key at
  runtime; this only turns the layer off. A `look.layer` you set in
  `settings` wins over the pinned one.
  ```nix title="home.nix"
  programs.scootbar.stylix.enable = false;
  ```

Your file and flags still win per key over the layer; every other
key follows it. The fragment carries its own face, and the
plain-default font stays out while the layer is active, so the
layer's face shows. The face is the regular file found in the Stylix
sans package (`DejaVuSans.ttf` for "DejaVu Sans",
`Family-Regular.ttf` for Noto or a Nerd Font). Variable fonts,
`.ttc` collections and odd namings are not resolved — the bar then
falls back to its default face with a build-time warning rather than
a failed build.

With a catalog present the option does nothing and warns at
evaluation: the bar follows the active look instead of a fixed Stylix
layer. Set it to `false` to silence the warning.

Precedence, highest first, per key: daemon flags, the file, the look
layer, the module's plain default.

The six tokens, and how icons follow them (path icons tint from the
theme; image icons keep their own colors), are in
[Colors](./configure.md#colors) and [Icons](./modules.md#icons).
