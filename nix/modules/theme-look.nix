# The one helper every desktop slot reads look-derived values through:
# the look behind `desktop.look` (null without one; the `enum` type
# guarantees the name, so the lookup cannot fail), whether a theme
# target applies, and the two hex spellings configs need. Later
# children read through here, never hand-mapping per-target colors
# out of `looks` the way `home.nix`/`scootbar.nix` used to.
#
# Takes only `lib`, like `desktop.nix` itself. Each function takes the
# `desktop` subtree (whatever module reads it: `config.programs.scoot`
# owns it on both scoot sides, the bar module reads it across the
# boundary with `or {}` defaults), never the whole config.
{ lib }:

let
  desktop = import ./desktop.nix { inherit lib; };
in
{
  # The look behind `desktop.look`, or null without one.
  lookFor =
    desktopCfg: if (desktopCfg.look or null) == null then null else desktop.looks.${desktopCfg.look};

  # Whether a theme target applies: a look is set, and its
  # `theme.targets.<name>.enable` is on. The `or true` keeps this
  # reading with an older `desktop` subtree that predates the target.
  themed =
    desktopCfg: name:
    (if (desktopCfg.look or null) == null then null else desktop.looks.${desktopCfg.look}) != null
    && ((desktopCfg.theme.targets.${name}.enable or true));

  # A `#rrggbb` look token without its `#` (swaylock, foot).
  noHash = color: lib.removePrefix "#" color;

  # A `#rrggbb` look token as opaque `RRGGBBAA` (fuzzel, wob).
  withAlpha = color: "${lib.removePrefix "#" color}ff";

  # The app colors derived from a look's palette: the accent apps
  # carry (the bar's own accent, so a selected row or slider reads as
  # the look's -- amber on moonrise, not Adwaita's default blue) and
  # the surfaces they sit on (the bar's background/foreground, so app
  # text contrast equals bar text contrast, already AA -- measured in
  # `nix/tests.nix`). `accentFg` is the background color: on all five
  # looks the background on the accent passes AA (6.2-7.0, ginger-night
  # 6.8) while the foreground on the accent fails (1.7-2.4), so dark
  # text sits on the
  # bright accent -- and a future look that breaks either pairing fails
  # the AA content check loudly instead of shipping pale-on-pale text.
  # Read by the GTK `gtk.css` and the Qt color scheme in
  # `theme-home.nix`, never hand-mapped per target.
  appColors = look: {
    accent = look.barColors.accent;
    accentFg = look.barColors.background;
    windowBg = look.barColors.background;
    windowFg = look.barColors.foreground;
    viewBg = look.barColors.background;
    viewFg = look.barColors.foreground;
    headerbarBg = look.barColors.background;
    headerbarFg = look.barColors.foreground;
    dim = look.barColors.dim;
  };

  # The one cursor for every look (Vanilla-DMZ: the classic X cursor,
  # 3.3 MiB unpacked against Bibata's 322 MiB -- measured on the
  # pinned rev -- so the fallback costs nothing). Same name and size
  # everywhere the theme names a cursor: the compositor config, GTK's
  # `settings.ini`, `XCURSOR_THEME`/`XCURSOR_SIZE` and the greeter.
  cursor = {
    name = "Vanilla-DMZ";
    size = 24;
  };

  # The one icon theme for every look (Adwaita, the toolkit default:
  # GTK and Qt both ship expecting it, so nothing is invented).
  iconTheme = "Adwaita";
}
