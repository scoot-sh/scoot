# The radial burst look

A dark, high-contrast look built around one wallpaper: colorful rays exploding
from a dark center. It is an **opt-in example**, not a default: nothing in scoot,
scootbar or scootbg changes unless you use these files.

![scoot with the radial burst look: a floating translucent bar with circled workspaces, two terminals with a blue ring on the focused one and a thin orange ring on the other, the wallpaper beside them](../../assets/radial-burst-preview.png)

| File | For |
| --- | --- |
| [`scoot.toml`](scoot.toml) | the compositor: background, ring colors and widths, rounded corners, column widths, the wallpaper |
| [`bar.toml`](bar.toml) | scootbar: a floating, rounded, translucent bar, workspaces as circles, the clock |
| [`foot.ini`](foot.ini) | foot: the palette, padding and font size (needs a foot with `[colors-dark]` sections, 1.24 or later) |

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
  three-window session under pixman; see [configuration.md](../../configuration.md#appearance)).
  `0` is free.
- **The translucent bar** (`opacity`) needs an ARGB buffer and no opaque region. On the
  Asahi M2 the shaped and translucent looks cost no more than a flush one that the
  method could resolve; see the [resource ratchet](../../scootbar/backlog/lightest.md#appearance-looks-flush-against-floating).
  Set `opacity = 1` (and `radius = 0`) for the plain bar.

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
