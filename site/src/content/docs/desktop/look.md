---
title: Look and theme
description: "Pick a look, theme every app from it, and fetch a wallpaper from a link."
---

## Pick a look

`look` names the declared default: one entry of `programs.scoot.looks`
(default: every shipped look — list paths to your own source look
directories beside the names to add them). Each look renders at build
time into the catalog (`~/.local/share/scoot/looks` sources beside
`~/.local/share/scoot/rendered` renders); each activation runs
`scoot look sync`, which starts a fresh state on the declared
default, switches when it changed, falls back when the live choice
left the catalog, and otherwise keeps the live choice. A swap
(`scoot look apply`, the look-cycle key, the IPC request) repoints
the state link and reloads each running app's look layer at once; the
declaration only wins when it changes. The default is `ginger-night`
(the cat-peeking look): a profile with no `look` choice renders it.

`programs.scoot.lookPackage` is the `scoot-look` binary that renders
the catalog at build time: a package or null (default:
`pkgs.scoot-look`, which the flake's overlay and module wrappers
provide — without them, point it at
`inputs.scoot.packages.${system}.scoot-look`). Null installs no
catalog: no sources, no renders, no declared default, no sync step.
It takes effect on the next build.

| Look | Compositor ring / background | Bar | Wallpaper |
|---|---|---|---|
| `ginger-night` (default) | ginger ring `#FF9A30`, near-black `#0E0E0E` | black, cream and ginger | ships (copied to the store; `fit` on the look's black, so the cat is never cropped) |
| `music-desk` | blue ring `#3D579A`, paper `#FCFBFB` | paper, ink and blue | ships (copied to the store) |
| `radial-burst` | blue ring `#31a9e5`, plum `#241721` | plum, yellow and blue | ships (copied to the store) |
| `moonrise` | amber ring `#FF9A49`, slate navy `#2B3648` | navy, cream and amber | ships (copied to the store) |
| `vinyl-sunset` | orange ring `#FE9949`, espresso `#270E0A` | espresso, cream and orange | ships (copied to the store) |

With the profile on and no `look` set, the look is `ginger-night`: that is the
default. `null` themes nothing and is only reached by setting `look = null`
explicitly (the opt-out), or by not enabling the profile at all. A look
is presentation only (colors, fonts, opacity, shape, cursor, the
wallpaper): layout stays yours — gaps, column widths, module lists,
binds. The shipped looks carry their shape (rounded windows, ring
widths, bar padding and spacing; the bar stays opaque), where the Nix
path used to apply only colors, fonts and the wallpaper. To keep
square windows, set `settings.appearance.corner_radius = 0`: your
file wins per key.

**Precedence**, highest first, per key: a value you set in `settings`,
then the active look's layer, then the app default. So
`settings.appearance.background_color = "#123456"` beside
`look = "music-desk"` replaces that one color and keeps the rest of the
look. What Stylix sets in `settings` (where present) sits above the
layer, so a live swap cannot change those keys. One combination is
invalid the same way the Stylix one is: a `wallpaper.color` you set
yourself beside a look's shipped `image` (the two together are refused
by scoot, fail-safe — the session carries on with `background_color`).
Set your own `image` instead (it wins per key: a `[wallpaper]` key of
yours keeps your whole section), or drop to `look = null`.

Every look is previewed in [Theming](../scoot/theming.md) — including
how to make your own.

## App theme

One look themes every app together: picking `music-desk` sets the
GTK theme, the Qt style, the cursor, the fonts and the dark-mode
preference from that look in one pass — no per-app copy-paste. Every
value is an option, applied on rebuild/switch (newly started apps
pick the theme up; running apps re-read GTK's `settings.ini` where
the toolkit watches it, the rest on restart — the greeter takes it
at the next login). Nothing here runs: files, packages and session
variables only, no daemon, so the theme costs no wakeups.

| Piece | Option | Type | Default | Notes |
|---|---|---|---|---|
| GTK apps | `desktop.theme.targets.gtk.enable` | bool | `true` | `settings.ini` (GTK 3 + 4): Adwaita, or Adwaita-dark for a dark look, with the look's proportional sans, cursor and dark preference — plus `gtk.css` carrying the look's accent and surfaces (named colors and the `:root` vars libadwaita paints with); the dark-mode signal is `color-scheme`, written by `scoot look sync` from the active look (no `GTK_THEME` force: it would make libadwaita skip its own providers) |
| Qt apps | `desktop.theme.targets.qt.enable` | bool | `true` | `qt6ct.conf` through `QT_QPA_PLATFORMTHEME=qt6ct` (plus `QT_PLUGIN_PATH` for its platformtheme and style): the Adwaita Qt style in the look's polarity, Adwaita icons, and a generated color scheme from the look's palette — dark looks get a dark palette, the light look a light one |
| Cursor | `desktop.theme.targets.cursor.enable` | bool | `true` | Mocu at its native 24 px: the look layer names it for the compositor plus `XCURSOR_THEME`/`XCURSOR_SIZE` for X11 apps — the same art as the built-in fallback, as real theme files; the managed sync renders no cursor keys while the target is off |
| Fonts | `desktop.theme.targets.fonts.enable` | bool | `true` | DejaVu Sans for sans-serif (proportional UI text in GTK/Qt apps and the greeter), FiraCode Nerd Font for monospace (DejaVu Sans Mono for `radial-burst`), through fontconfig — and the look layer carries the bar's DroidSansM Nerd Font Propo file, whose glyphs carry the module icons; the managed sync renders no bar font while the target is off. The profile installs all three families (NixOS system-wide, home-manager per user with `fonts.fontconfig.enable` on by default) so `scoot look apply` resolves them; a family still missing falls back with a note instead of refusing |
| Login screen | `desktop.theme.targets.greeter.enable` | bool | `true` | the session wallpaper behind the login card, the dark setting, the look's ReGreet CSS and font — needs `greeter.enable` too |
| Terminal | `desktop.theme.targets.terminal.enable` | bool | `true` | the look's `foot.ini` from the flake |
| Prompt | `desktop.theme.targets.shell.enable` | bool | `true` | the look's `starship.toml` (`radial-burst` ships none: inert there) |
| Editor | `desktop.theme.targets.editor.enable` | bool | `true` | the look's Helix config beside its theme (`radial-burst` ships none: inert there) — plus the look's Neovim colorscheme, linked under its fixed name and loaded on demand, never selected |
| Editor select | `desktop.theme.targets.editor.nvim.enable` | bool | `false` | select the linked Neovim colorscheme at startup (a `plugin/` file running `colorscheme scoot-look`); off leaves the scheme loaded but unselected, and home-manager's `programs.neovim` skips the selection (select the scheme in your own init) |
| Monitor | `desktop.theme.targets.monitor.enable` | bool | `true` | the look's btop config beside its theme (`radial-burst` ships none, `music-desk` ships no theme: inert there) |
| Input method | `desktop.theme.targets.inputMethod.enable` | bool | `true` | a generated `scoot-look` classic-UI theme (background and text, selection and border from the look's palette) plus the `classicui.conf` naming it — needs `inputMethod.enable` too |
| Extra GTK keys | `desktop.theme.settings` | attrs of str | `{ }` | merged over the generated `settings.ini`: a value here wins per key |

On NixOS the profile also turns on `programs.dconf.enable` (as a
default, so your own `false` wins): `scoot look sync` writes the
dark-mode signal into dconf after the links land, and its activation
needs the system's dconf service to do that. With the profile on only
in your home config, set `programs.dconf.enable = true;` in the
system config yourself.

Set any target to `false` to keep that piece's own style while the
rest follows the look — for example, to leave alone an app you
already themed:

```nix title="home.nix"
programs.scoot.desktop.theme.targets.gtk.enable = false;  # my GTK is hand-tuned
```

The cursor is [Mocu](https://github.com/sevmeyer/mocu-xcursor): the profile
installs it as real XCursor files (`Mocu`, white at its native 24 px, with
`Mocu-Black` beside it in the same package — see
`nix/mocu-cursor-theme.nix`), the look layer names it for the compositor,
and the theme exports it for X11/XWayland apps through
`XCURSOR_THEME`/`XCURSOR_SIZE`. It is the
same art scoot draws as its built-in fallback cursors (see
[Appearance](../scoot/appearance.md#the-built-in-cursors)), so the fallback
rarely shows, and when it does it is the same family. The busy cursors are
static first frames in both: Mocu's animation frames are not vendored. To use
another theme, point `desktop.theme.cursor.package` at it and name it in
`settings.appearance.cursor_theme` (a value there wins per key); set
`targets.cursor.enable = false` to leave the cursor alone entirely.

If you theme an app through its own home-manager module instead
(`programs.foot`, `programs.starship`, `programs.helix`,
`programs.btop`), that module owns the same file and wins: the
look's file stays out, so the two never conflict. The same holds for
GTK and Qt, whose modules live at the top level rather than under
`programs`: `gtk.enable` owns both `settings.ini` files — and only
what it actually writes besides them (its `extraCss` or theme owns
`gtk.css`, its `colorScheme` the dconf `color-scheme` leaf) — while
`qt.enable` with `qt6ctSettings` owns `qt6ct.conf`. The look's file
stays out of each owned path while the rest stays themed, so enabling
the upstream module beside the profile evaluates cleanly with your
file winning.

**Precedence**, highest first, per key: a value you set (in
`settings`, in `theme.settings`, or in the app's own module), then the
active look's layer, then the toolkit default. What Stylix sets in
`settings` (where present) sits above the layer, so a live swap cannot
change those keys. So `theme.settings.gtk-theme-name
= "HighContrast"` beside `look = "music-desk"` replaces that one key
and keeps the rest of the look.

`QT_PLUGIN_PATH` composes instead of replacing: the systemd user
session (`environment.d`) holds one string per variable, so a value
you set there wins whole — the same replacement home-manager's own
Qt module uses for its profile dirs. Shells get both: the theme
publishes its plugin dirs through `home.sessionSearchVariables`,
which home-manager prepends before your own `$QT_PLUGIN_PATH` at
login, so an input-method path you export keeps working beside the
theme's qt6ct and style dirs. Put extra plugin dirs in
`home.sessionSearchVariables` to compose everywhere shells reach.

> **Symptom:** a Qt app ignores the theme (default fusion look, wrong
> colors).
>
> Check `QT_QPA_PLATFORMTHEME` in the app's environment
> (`systemctl --user show-environment | grep QT_QPA`) and that
> `qt6ct` plus `adwaita-qt6` are installed (`nix-store -q --requisites`
> will not list them if `targets.qt` is off or its packages are
> null). A light palette inside a dark look means the scheme did not
> apply: check `custom_palette=true` and `color_scheme_path` in
> `~/.config/qt6ct/qt6ct.conf` point at the generated
> `colors/scoot-look.conf`. If you export your own `QT_PLUGIN_PATH`
> (an input method, say) and Qt apps lose the theme in some launcher
> but not in a terminal, your manager-level value is winning whole
> over the theme's dirs there — move it to
> `home.sessionSearchVariables` so shells compose both. Apps started before the switch keep the old theme —
> restart them.

> **Symptom:** a libadwaita app stays light under a dark look (or dark
> under the light one).
>
> The look's dark mode reaches libadwaita through dconf
> (`org.gnome.desktop.interface color-scheme`), not through
> `settings.ini` alone — read it back with `dconf read
> /org/gnome/desktop/interface/color-scheme` (dark looks say
> `'prefer-dark'`, the light look `'prefer-light'`). If the key is
> right and the app is still wrong, it started before the switch —
> restart it. `gtk-application-prefer-dark-theme` in `settings.ini`
> stays for plain GTK and older libadwaita; the pinned libadwaita only
> warns on it.

> **Symptom:** a Flatpak app ignores the theme.
>
> The sandbox hides your config: give it the files with
> `flatpak override --filesystem=xdg-config/gtk-3.0:ro
> --filesystem=xdg-config/gtk-4.0:ro <app-id>` (and the theme
> packages with `--filesystem=/nix/store:ro` on NixOS), or install
> the matching `org.gtk.Gtk3theme.*` extension. The look's
> fontconfig and cursor need the same treatment.

X11 apps (XWayland) read the same `settings.ini` and `qt6ct.conf` —
no settings daemon runs for them. The theme deliberately sets no
`Xft.dpi`: the compositor publishes the output scale and XWayland
follows it, and a theme dpi would fight that.

## Look-independent effects

Effects (shadow, fill, motion, card style, title weight) belong to the look,
and the daemons also take them from their own files, where they win per key
over any look. `desktop.theme.overrides` writes those file tables for you, so
one setting applies to every look, shipped or your own:

```nix title="home.nix"
programs.scoot.desktop.theme.overrides = {
  effects = { shadow = "soft"; motion = "spring"; };
  card.mark = "stripe";
  title.weight = 600;
};
```

| Option | Type / default | Written to |
|---|---|---|
| `desktop.theme.overrides.effects` | attrs of strings, numbers and booleans, `{ }` | `[effects]` of `notify.toml` and `launch.toml` |
| `desktop.theme.overrides.card` | same | `[card]` of `notify.toml` |
| `desktop.theme.overrides.title` | same | `[title]` of `notify.toml` |

The keys and values are the ones in the notifier's
[Effects over any look](../scootnotify/configure.md#effects-over-any-look)
and the launcher's
[Effects over any look](../scootlaunch/panel.md#effects-over-any-look); the
daemons check them, and a bad value is ignored with a log line. A table left
empty writes nothing, so a profile that sets none renders the same files as
before. Each key is set at `mkDefault`, so `programs.scootnotify.settings.effects.motion`
(or `programs.scootlaunch.settings`) still wins for that key. Changing an
option reloads the running notifier in place; the launcher reads it at its
next open.

The launcher's file is written only when `programs.scootlaunch` is imported
(`homeModules.scootlaunch` or `nixosModules.scootlaunch`) and enabled; the
profile does not import it. `card` and `title` never reach `launch.toml`.

> **Symptom:** the overrides are set and the launcher ignores them.
>
> `programs.scootlaunch.enable` is off, or its module is not imported, so no
> `launch.toml` is generated. Import the module and enable it (the profile
> enables it for you when `desktop.launcher.daemon = "scootlaunch"`).

## Wallpaper from a link

The session wallpaper ([scootbg](../scootbg/index.md)) can be a link:
point `set` — or scoot's `[wallpaper] image` — at an `http(s)` URL and
the daemon downloads it once, caches it under `~/.cache/scootbg/`, and
shows it like any other image. Pin it with `sha256` so a changed byte
fails loudly instead of landing on screen:

```nix title="home.nix"
programs.scoot.settings.wallpaper = {
  image = "https://example.com/hills.jpg";
  sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
  mode = "fill";
};
```

Until the download lands — and when it fails — the compositor's
background shows and the error says why; nothing blocks and nothing
retries in a loop. Full detail (cache layout, the retry rules, every
failure mode) is in [Wallpaper from a link](../scootbg/from-url.md).
