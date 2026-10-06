---
title: Theming
description: "Pick one of the four looks, see what each file does, and make your own."
---

Make the whole desktop match one look: compositor ring and
background, bar colors, session wallpaper, GTK and Qt apps, cursor,
fonts, dark mode, the login screen, and the terminal, prompt, editor
and monitor configs — from one `look` choice. Set it in [the desktop
profile](../desktop/index.md#pick-a-look):

```nix
programs.scoot.desktop = {
  enable = true;
  look = "music-desk";   # "vinyl-sunset" | "radial-burst" | "moonrise" | null (no theming)
};
```

| Look | Ring / background | Wallpaper | |
|---|---|---|---|
| `music-desk` | blue `#3D579A` on paper `#FCFBFB` | ships | ![music-desk](../../../assets/music-desk-preview.png) |
| `radial-burst` | blue `#31a9e5` on plum `#241721` | ships | ![radial-burst](../../../assets/radial-burst-preview.png) |
| `moonrise` | amber `#FF9A49` on slate navy `#2B3648` | ships | ![moonrise](../../../assets/moonrise-preview.png) |
| `vinyl-sunset` | orange `#E59560` on espresso `#271A1F` | flat color (the illustration is yours to download) | ![vinyl-sunset](../../../assets/vinyl-sunset-preview.png) |

(Previews: real screenshots of each example, captured from live
sessions. The vinyl-sunset wallpaper illustration stays under its
Pixabay license — the example's `[wallpaper]` table points at a copy
you download, never a committed file.)

## What one look sets

Every row is a default a value you set wins over per key, and Stylix
wins over where present (your explicit value > Stylix > the look).
Each example is one directory with the same shape — `scoot.toml` (the
compositor: layout, appearance, binds), `bar.toml` (the bar),
`regreet.css` (the login screen), plus terminal and tool configs that
follow the palette:

- [vinyl-sunset](https://github.com/scoot-sh/scoot/tree/main/docs/examples/vinyl-sunset)
- [music-desk](https://github.com/scoot-sh/scoot/tree/main/docs/examples/music-desk)
- [radial-burst](https://github.com/scoot-sh/scoot/tree/main/docs/examples/radial-burst)
- [moonrise](https://github.com/scoot-sh/scoot/tree/main/docs/examples/moonrise)

| File the profile writes | From the look | Takes effect |
|---|---|---|
| `~/.config/gtk-3.0/settings.ini`, `~/.config/gtk-4.0/settings.ini` | Adwaita (Adwaita-dark for a dark look), Adwaita icons, the look's UI face and cursor, the dark preference | newly started apps; running GTK apps re-read it where the toolkit watches |
| `~/.config/qt6ct/qt6ct.conf` (through `QT_QPA_PLATFORMTHEME=qt6ct`) | the Adwaita Qt style in the look's polarity, Adwaita icons | Qt apps on restart |
| `~/.config/fontconfig/conf.d/10-scoot-look.conf` | the UI face for sans-serif, the terminal face for monospace | `fc-cache` is not needed: fontconfig reads it on next lookup |
| `~/.config/foot/foot.ini` | the example's own file | foot on restart |
| `~/.config/starship.toml` | the example's own file | next prompt |
| `~/.config/helix/config.toml` + `themes/<look>.toml` | the example's own files | Helix on restart |
| `~/.config/btop/btop.conf` + `themes/<look>.theme` | the example's own files | btop on restart |
| ReGreet (NixOS, needs `greeter.enable`) | the session wallpaper behind the login card, the dark setting, the look's CSS and UI face | next login |

`radial-burst` ships no shell, editor or monitor files, so those
three targets are inert for it; `music-desk` ships a btop config with
no theme file; `vinyl-sunset` pairs no greeter backdrop (its
illustration cannot be committed or auto-fetched under its license).

Per-target opt-out is Stylix-style, under one namespace, one flag per
themed piece (every target defaults on) — see the [option
table](../desktop/index.md#app-theme) for the full list:

```nix
programs.scoot.desktop.theme.targets.lock.enable = false;  # keep swaylock's own style
```

`desktop.bar.enable` keeps its meaning ("manage the bar at all") and
is never a theme switch.

## What stays a static file, and why

Generated from the look's palette where the format is mechanical:
`settings.ini`, `qt6ct.conf`, the fontconfig snippet, the ReGreet
dark setting, the cursor and font names. Installed as the example's
own static file where the palette is hand-tuned: `foot.ini`,
`starship.toml`, the Helix config and theme, the btop config and
theme, and `regreet.css`. A terminal palette is sixteen hand-placed
hues, not six bar roles projected wider — generating it would invent
colors no one chose, so the flake applies the file instead of
synthesizing one.

Qt follows polarity, font and icons rather than a per-look accent
for the same reason: no custom palette is forced, so a Qt app reads
as the look's dark or light sibling, not a stranger.

## Dark mode, X11 and Flatpak

Each look declares dark or light (`music-desk` light, the other
three dark): that drives `gtk-application-prefer-dark-theme`, the
Adwaita variant on both toolkits, and the greeter's dark setting.
X11 apps through XWayland read the same `settings.ini` and
`qt6ct.conf` — no settings daemon runs for them, and the theme sets
no `Xft.dpi`: the compositor publishes the output scale and XWayland
follows it. Flatpak apps need the config files exposed into the
sandbox — see the [Flatpak
symptom](../desktop/index.md#app-theme).

## Stylix

When `config.lib.stylix` exists and `stylix.enable` is on,
`programs.scoot.settings` gets defaults from it — the compositor's
half of a themed desktop, next to the bar's. The module never imports
Stylix; nothing changes without it, and
`programs.scoot.stylix.enable = false` turns the defaults off with it
present. (Home-manager only: the NixOS module renders no config file.)

| Setting | From | Notes |
|---|---|---|
| `appearance.focus_ring_active_color` | base16 `base0D` | The focused ring. |
| `appearance.focus_ring_inactive_color` | base16 `base03` | Every other ring. |
| `appearance.background_color` | base16 `base00` | The frame clear color. |
| `appearance.cursor_theme` | `stylix.cursor.name` | Only when `stylix.cursor` is set. |
| `appearance.cursor_size` | `stylix.cursor.size` | Only when `stylix.cursor` is set. |
| `wallpaper.image` | `stylix.image` | Only when set; setting it is what turns `wallpaper.enable` on. |
| `wallpaper.mode` | `stylix.imageScalingMode` | Only when set (`fill`, `fit`, `stretch`, `center`, `tile`). |

Precedence per key: your `settings` value, then Stylix's, then the
compositor default. One combination is invalid: your own
`wallpaper.color` plus Stylix's `image` (scoot takes `image` or
`color`, never both) — for a solid color under Stylix, set
`programs.scoot.stylix.wallpaper.enable = false`, set your own
`image`, or turn `stylix.enable` off. `cursor_color` has no Stylix
convention and stays at the compositor default.

A direct-module setup without the flake's overlay has no `scootbg`
package: the wallpaper default then stays off rather than failing.
Set `programs.scoot.wallpaper.enable = false` explicitly to own the
wallpaper daemon yourself (and point `settings.wallpaper.command`
at it, or leave the section to find `scootbg` on `PATH`).

## Make a look

A look is data, so new ones are cheap: one directory with the same
shape as the four above — palette, `scoot.toml`, `bar.toml`,
`regreet.css`, and a wallpaper (an image, or a palette color for a
license-clean look like vinyl-sunset's flat espresso). Wire its six
roles plus its dark/light polarity and its two font faces into the
flake's look registry (`nix/modules/desktop.nix`), and a check
renders every look into every target and fails on a missing role.

Where looks are headed (the standing direction, not all of it
shipped): the look list reads from a registry rather than a
hand-written enum, so adding a look means adding its directory;
`look = "auto"` matches your own wallpaper (through Stylix's palette
with Stylix present, else derived from the image at build time); and
a user-passed look (a path or attrset with the same shape) works the
same way. `look` only ever gains values — existing configs keep
working.
