# The radial burst look — dark

A dark, high-contrast look built around one wallpaper: colorful rays exploding
from a dark center. It is an **opt-in example**, not a default: nothing in scoot,
scootbar or scootbg changes unless you use these files.

![scoot with the radial burst look: one translucent terminal on the left running fastfetch with a thin blue ring, the wallpaper showing through it and filling the right half, under a floating translucent bar with circled workspaces, the window title, status icons and the power menu](../../assets/radial-burst-preview.png)

| File | For |
| --- | --- |
| [`scoot.toml`](scoot.toml) | the compositor: background, ring colors and widths, rounded corners, column widths with a full-width column on `Super+m`, the wallpaper |
| [`bar.toml`](bar.toml) | scootbar: a floating, rounded, translucent bar, workspaces as circles with the window title, the clock, status modules (WiFi, volume, brightness, bluetooth, battery), the power menu |
| [`foot.ini`](foot.ini) | foot: the palette, 80% opacity, padding and font size (needs a foot with `[colors-dark]` sections, 1.26 or later) |
| [`regreet.css`](regreet.css) | ReGreet: the login screen in plum, ray blue and burst orange (NixOS greeter) |

It needs `scootbg` on your `PATH` for the wallpaper (scoot starts it itself; the Nix
modules install it when the settings have a `[wallpaper]` table, from source
`cargo install --path crates/scootbg`), and a font file
for scootbar (see the comment in [`bar.toml`](bar.toml)). Without `scootbg` scoot logs one
warning and carries on with the background color.

Try it from a checkout, each in its own terminal (scootbar and foot once scoot is up):

```sh
scoot --config docs/examples/radial-burst/scoot.toml
scootbar daemon --config docs/examples/radial-burst/bar.toml
foot --config docs/examples/radial-burst/foot.ini
```

To keep it, copy the files to your config directory (`~/.config/scoot/config.toml`,
`~/.config/scoot/bar.toml`, `~/.config/foot/foot.ini`) and point `[wallpaper] image` at
a copy of the wallpaper.

## The login screen

On NixOS the greeter can wear this look too. Copy `regreet.css` and
`docs/assets/wallpapers/radial-burst.png` next to your system configuration:

```nix
programs.scoot = {
  enable = true;
  greeter = {
    enable = true;
    background = ./radial-burst.png;
  };
};
services.displayManager.regreet = {
  extraCss = ./regreet.css;
  font = {
    package = pkgs.nerd-fonts.droid-sans-mono;
    name = "DroidSansM Nerd Font Propo";
    size = 12;
  };
  settings = {
    GTK.application_prefer_dark_theme = true;
    background.fit = "Cover";
    widget.clock.format = "%-I:%M %P";
  };
};
```

This sheet was mixed from the palette without a live greeter in front of it
(see the note at its top); expect small tuning. See [the greeter
docs](../../nix.md#the-greeter-regreet-opt-in) for the one-screen default and
the other knobs.

## The palette

The image's hues: background `#241721` (plum), blue `#31a9e5`, two oranges (`#e36e38`
for the ring, `#fa9233` in foot), hot pink `#bf128d`, purple `#6d1d98`, yellow `#fdef1d`
and olive `#99911d`.
The focused window's ring is blue and the others' orange; the bar, foot and scoot all
read from the same set. They came from Stylix's palette generator run on the image,
with two changes by hand: the terminal's ANSI slots are spread across the six hues (the
generator reuses a few for several slots, so blue and cyan came out the same), and
scoot's ring colors are set here because scoot's Nix module has no Stylix defaults.

## What it costs

The defaults stay light on purpose; this look spends some of that, so each piece is
opt-in and you can drop any of them:

- **The wallpaper** makes scootbg hold the decoded image: about 12 MB RSS at 1920x1080
  and about 37 MB on two 4K outputs, against about 4 MB for a solid color (measured with
  a larger JPEG, but the buffer follows the output, not the file;
  [scootbg](../../scootbg/README.md)). Drop `[wallpaper]` for a solid
  `background_color` and the cost is gone.
- **`corner_radius`** costs a little per frame when non-zero (measured at about +9% on a
  three-window session under pixman; see [appearance](https://scoot-sh.github.io/scoot/scoot/appearance.md)).
  `0` is free.
- **The translucent bar** (`opacity`) needs an ARGB buffer and no opaque region. On the
  Asahi M2 the shaped and translucent looks cost no more than a flush one that the
  method could resolve; see the [resource ratchet](../../scootbar/backlog/lightest.md#appearance-looks-flush-against-floating).
  Set `opacity = 1` (and `radius = 0`) for the plain bar.
- **The translucent terminals** (foot's `alpha=0.80`) make each terminal window non-opaque,
  so scoot blends it over the wallpaper whenever that region is redrawn. In the optional
  `gpu-scanout` `--tty` tier, a fullscreen translucent window also cannot be handed to the
  display directly ([backends](https://scoot-sh.github.io/scoot/scoot/backends.md)). That extra work was not measured here. Set
  `alpha=1.0` in [`foot.ini`](foot.ini) for opaque terminals.

## Image credit and license

`docs/assets/wallpapers/radial-burst.png` is "Colorful radial lines exploding on a dark
background" by Sufyan pir, published on Unsplash on June 18, 2026:
<https://unsplash.com/illustrations/colorful-radial-lines-exploding-on-a-dark-background-ETTtKnva9MM>.
Its page (checked 2026-09-30) says "Free to use under the Unsplash License"
(<https://unsplash.com/license>), which lets you download, copy, modify and distribute
it, commercially and without attribution, but not compile images to build a similar or
competing service; the credit here is a courtesy. **The image is not covered by this
repository's MIT license**: it stays under Unsplash's. If you redistribute scoot or fork
it, it comes with that license. To drop it, remove the file and the `[wallpaper]` table.
See [NOTICE](../../../NOTICE).
