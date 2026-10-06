# The moonrise look — chill

A calm, dark look built around one illustration: black tree silhouettes under
a huge amber disc, in a night sky fading from slate navy through mauve to
dusty rose. It is an **opt-in example**, not a default: nothing in scoot,
scootbar or scootbg changes unless you use these files.

![scoot with the moonrise look: a floating translucent bar with circled workspaces and the focused window's title over two translucent terminal columns, the night-sky illustration showing through](../../assets/moonrise-preview.png)

| File | For |
| --- | --- |
| [`scoot.toml`](scoot.toml) | the compositor: background, ring colors and widths, rounded corners, gaps, the wallpaper |
| [`bar.toml`](bar.toml) | scootbar: a floating, rounded, translucent bar, workspaces as discs with the focused window's title, the clock, system modules, two command-fed modules, launcher buttons and the power menu |
| [`foot.ini`](foot.ini) | foot: the palette, 80% opacity, padding and font size (needs a foot with `[colors-dark]` sections, 1.26 or later) |
| [`starship.toml`](starship.toml) | starship: the prompt in the illustration's palette |
| [`helix/config.toml`](helix/config.toml) | Helix: relative line numbers, cursor line, the theme below |
| [`helix/themes/scoot-moonrise.toml`](helix/themes/scoot-moonrise.toml) | Helix: the transparent palette theme |
| [`btop/btop.conf`](btop/btop.conf) | btop: a dark transparent setup naming the theme below |
| [`btop/themes/moonrise.theme`](btop/themes/moonrise.theme) | btop: the night-sky palette theme |
| [`lazygit.yml`](lazygit.yml) | lazygit: the palette, rounded borders |
| [`load.sh`](load.sh), [`cpu.sh`](cpu.sh) | the bar's `load` and `cpu` modules: load average and CPU percent on stdout |
| [`regreet.css`](regreet.css) | ReGreet: the login screen in slate navy, cream and disc amber (NixOS greeter) |

It needs `scootbg` on your `PATH` for the wallpaper (scoot starts it itself; the Nix
modules install it when the settings have a `[wallpaper]` table, from source
`cargo install --path crates/scootbg`), a font file
for scootbar (see the comment in [`bar.toml`](bar.toml)), and `load.sh` and
`cpu.sh` somewhere on your `PATH` (see the comment in [`bar.toml`](bar.toml)).
Without `scootbg` scoot logs one
warning and carries on with the background color. The rest is optional:
starship, Helix, btop and lazygit each read their file only if you run them;
btop needs the theme copied beside its config (see below).

Try it from a checkout, each in its own terminal (scootbar and foot once scoot is up):

```sh
scoot --config docs/examples/moonrise/scoot.toml
PATH="$PWD/docs/examples/moonrise:$PATH" scootbar daemon --config docs/examples/moonrise/bar.toml
foot --config docs/examples/moonrise/foot.ini
```

To keep it, copy the files to your config directory (`~/.config/scoot/config.toml`,
`~/.config/scoot/bar.toml`, `~/.config/foot/foot.ini`,
`~/.config/starship.toml`, `~/.config/helix/`, `~/.config/btop/btop.conf`
with `btop/themes/moonrise.theme` as `~/.config/btop/themes/moonrise.theme`,
`~/.config/lazygit/config.yml`) and point `[wallpaper] image` at
a copy of the wallpaper. The prompt in the preview runs under
`STARSHIP_CONFIG` pointing at `starship.toml`, e.g.
`env STARSHIP_CONFIG=~/.config/starship.toml bash -i`.

## The login screen

On NixOS the greeter can wear this look too. Copy `regreet.css` and
`docs/assets/wallpapers/moonrise.png` next to your system configuration:

```nix
programs.scoot = {
  enable = true;
  greeter = {
    enable = true;
    background = ./moonrise.png;
  };
};
services.displayManager.regreet = {
  extraCss = ./regreet.css;
  font = {
    package = pkgs.dejavu_fonts.minimal;
    name = "DejaVu Sans";
    size = 12;
  };
  settings = {
    GTK.application_prefer_dark_theme = true;
    background.fit = "Cover";
    widget.clock.format = "%-I:%M %P";
  };
};
```

See [the greeter docs](../../nix.md#the-greeter-regreet-opt-in) for the
one-screen default and the other knobs.

## The palette

Role-named, so the coming look registry can absorb this look without rework:

| Role | Color | Used for |
| --- | --- | --- |
| `surface` | slate navy `#2B3648` | compositor background, bar background |
| `surface-deep` | darker navy `#232C3B` | terminal background |
| `ink` | cream `#F6EEDC` | body text everywhere (bar, terminal, Helix, btop, lazygit) |
| `accent` | disc amber `#FFA45C` | focused ring (`#FF9A49`, the disc's own mid-tone), bar accent, prompt directory |
| `glow` | disc yellow `#FFD54A` | hover, prompt success, btop highlights |
| `hush` | mauve `#5E4B5B` | inactive ring, entry fields |
| `mist` | pale mauve `#9C8B95` | dim text, separators, line numbers |
| `ember` | dusty rose `#E87F6A` | urgent, errors, deletions |
| `dusk` | dusty mauve `#B595AD` | inactive workspace numbers, git branch |
| `ridge` | slate blue `#7C8FB0` | ANSI blue/cyan family (the mountain) |
| `moss` | muted sage `#8FA382` | ANSI green family (kept desaturated to stay chill) |

Body text is cream on slate navy at **10.6:1**, amber at **6.2:1** and
yellow at **8.6:1** (WCAG AA needs 4.5:1). `mist` (3.8:1) and the inactive
ring never carry body text — only separators, dimmed numbers and unfocused
rings, the same role vinyl-sunset's dim taupe plays.

## What it costs

The defaults stay light on purpose; this look spends some of that, so each piece is
opt-in and you can drop any of them:

- **The wallpaper** makes scootbg hold the decoded image: about 12 MB RSS at 1920x1080
  and about 37 MB on two 4K outputs, against about 4 MB for a solid color (measured with
  a larger JPEG, but the buffer follows the output, not the file;
  [scootbg](../../scootbg/README.md)). Drop `[wallpaper]` for a solid
  `background_color` and the cost is gone.
- **`corner_radius`** costs a little per frame when non-zero (measured at about +9% on a
  three-window session under pixman; see [appearance](https://www.scoot.sh/scoot/appearance.md)).
  `0` is free.
- **The translucent bar** (`opacity`) needs an ARGB buffer and no opaque region. On the
  Asahi M2 the shaped and translucent looks cost no more than a flush one that the
  method could resolve; see the [resource ratchet](../../scootbar/backlog/lightest.md#appearance-looks-flush-against-floating).
  Set `opacity = 1` (and `radius = 0`) for the plain bar.
- **The translucent terminals** (foot's `alpha=0.80`) make each terminal window non-opaque,
  so scoot blends it over the wallpaper whenever that region is redrawn. In the optional
  `gpu-scanout` `--tty` tier, a fullscreen translucent window also cannot be handed to the
  display directly ([backends](https://www.scoot.sh/scoot/backends.md)). That extra work was not measured here. Set
  `alpha=1.0` in [`foot.ini`](foot.ini) for opaque terminals.
- **The `load` and `cpu` modules** each hold one shell that sleeps (10 s and 5 s) and
  prints a line: no polling by the bar, one process each, woken by their own timers.

## Image credit and license

`docs/assets/wallpapers/moonrise.png` is "Silhouetted trees under moon and
stars" by saatvik 5554 (@saatvik_reddy_suravaram), published on Unsplash:
<https://unsplash.com/illustrations/silhouetted-trees-under-moon-and-stars-jwBJOj6gakI>.
Its page says "Free to use under the Unsplash License"
(<https://unsplash.com/license>), which lets you download, copy, modify and distribute
it, commercially and without attribution, but not compile images to build a similar or
competing service; the credit here is a courtesy. **The image is not covered by this
repository's MIT license**: it stays under Unsplash's. If you redistribute scoot or fork
it, it comes with that license. To drop it, remove the file and the `[wallpaper]` table.
See [NOTICE](../../../NOTICE).
