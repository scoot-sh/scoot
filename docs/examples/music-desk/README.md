# The music desk look — light

A light, paper-white look built around one wallpaper: musical instruments on a
white surface. It is an **opt-in example**, not a default: nothing in scoot,
scootbar or scootbg changes unless you use these files.

![scoot with the music desk look: an edge-to-edge translucent bar with circled workspaces and the focused window's title over three translucent terminal columns (fastfetch, Helix on scoot's source, btop over lazygit), the wallpaper showing through](../../assets/music-desk-preview.png)

| File | For |
| --- | --- |
| [`scoot.toml`](scoot.toml) | the compositor: background, ring colors and widths, rounded corners, gaps, the wallpaper |
| [`bar.toml`](bar.toml) | scootbar: an edge-to-edge translucent bar, workspaces as discs with the focused window's title, the clock, system modules, two command-fed modules, launcher buttons and the power menu |
| [`foot.ini`](foot.ini) | foot: the palette, 80% opacity, padding and font size |
| [`starship.toml`](starship.toml) | starship: the prompt in the wallpaper's palette |
| [`helix/config.toml`](helix/config.toml) | Helix: relative line numbers, cursor line, the theme below |
| [`helix/themes/scoot-light.toml`](helix/themes/scoot-light.toml) | Helix: the transparent palette theme |
| [`btop.conf`](btop.conf) | btop: a light transparent theme |
| [`lazygit.yml`](lazygit.yml) | lazygit: the palette, rounded borders |
| [`load.sh`](load.sh), [`cpu.sh`](cpu.sh) | the bar's `load` and `cpu` modules: load average and CPU percent on stdout |
| [`regreet.css`](regreet.css) | ReGreet: the login screen in paper, ink and headphone blue (NixOS greeter) |

It needs `scootbg` on your `PATH` for the wallpaper (scoot starts it itself; the Nix
modules install it when the settings have a `[wallpaper]` table, from source
`cargo install --path crates/scootbg`), a font file
for scootbar (see the comment in [`bar.toml`](bar.toml)), and `load.sh` and
`cpu.sh` somewhere on your `PATH` (see the comment in [`bar.toml`](bar.toml)).
Without `scootbg` scoot logs one
warning and carries on with the background color. The rest is optional:
starship, Helix, btop and lazygit each read their file only if you run them.

Try it from a checkout, each in its own terminal (scootbar and foot once scoot is up):

```sh
scoot --config docs/examples/music-desk/scoot.toml
PATH="$PWD/docs/examples/music-desk:$PATH" scootbar daemon --config docs/examples/music-desk/bar.toml
foot --config docs/examples/music-desk/foot.ini
```

To keep it, copy the files to your config directory (`~/.config/scoot/config.toml`,
`~/.config/scoot/bar.toml`, `~/.config/foot/foot.ini`,
`~/.config/starship.toml`, `~/.config/helix/`, `~/.config/btop/btop.conf`,
`~/.config/lazygit/config.yml`) and point `[wallpaper] image` at
a copy of the wallpaper. The prompt in the preview runs under
`STARSHIP_CONFIG` pointing at `starship.toml`, e.g.
`env STARSHIP_CONFIG=~/.config/starship.toml bash -i`.

## The login screen

On NixOS the greeter can wear this look too. Copy `regreet.css` and
`docs/assets/wallpapers/music-desk.png` next to your system configuration:

```nix
programs.scoot = {
  enable = true;
  greeter = {
    enable = true;
    background = ./music-desk.png;
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
    background.fit = "Cover";
    widget.clock.format = "%-I:%M %P";
  };
};
```

A light look, so no dark-theme switch. See [the greeter
docs](../../nix.md#the-greeter-regreet-opt-in) for the one-screen default and
the other knobs.

## The palette

The wallpaper's colors: paper `#FCFBFB`, ink `#1A2032`, headphone blue `#3D579A`
(accent and focused ring), hover `#5D7AB0`, inactive ring `#D5D7DD`, dim `#C9CBD0`,
guitar red (`#E0574A`, bright `#EE6F5E`), amp green (`#5F7D3F`, bright `#7A9A55`),
cassette purple (`#8A73C4`), record yellow (`#C9962A`, bright `#E6B440`), teal
`#3F8A9A`. The focused window's ring is blue and the others' pale grey; the bar,
foot, starship, Helix, btop and lazygit are all themed from these hues.

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
  Set `opacity = 1` for the plain bar.
- **The translucent terminals** (foot's `alpha=0.80`) make each terminal window non-opaque,
  so scoot blends it over the wallpaper whenever that region is redrawn. In the optional
  `gpu-scanout` `--tty` tier, a fullscreen translucent window also cannot be handed to the
  display directly ([backends](https://scoot-sh.github.io/scoot/scoot/backends.md)). That extra work was not measured here. Set
  `alpha=1.0` in [`foot.ini`](foot.ini) for opaque terminals.
- **The `load` and `cpu` modules** each hold one shell that sleeps (10 s and 5 s) and
  prints a line: no polling by the bar, one process each, woken by their own timers.

## Image credit and license

`docs/assets/wallpapers/music-desk.png` is "Musical instruments and audio equipment
on a white surface" by Alghozy (@artgho), published on Unsplash:
<https://unsplash.com/illustrations/musical-instruments-and-audio-equipment-on-a-white-surface-b6Us5E-BO8w>.
Its page says "Free to use under the Unsplash License"
(<https://unsplash.com/license>), which lets you download, copy, modify and distribute
it, commercially and without attribution, but not compile images to build a similar or
competing service; the credit here is a courtesy. **The image is not covered by this
repository's MIT license**: it stays under Unsplash's. If you redistribute scoot or fork
it, it comes with that license. To drop it, remove the file and the `[wallpaper]` table.
See [NOTICE](../../../NOTICE).
