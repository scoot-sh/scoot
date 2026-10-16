# The one helper every desktop slot reads look-derived values through:
# the registry look behind `programs.scoot.look` (null without one, or
# without a registry entry for the name — a custom catalog look leaves
# the eval-time leaves inert until the leaves tickets move each one onto
# the rendered look), whether a theme target applies, and the two hex
# spellings configs need. Later children read through here, never
# hand-mapping per-target colors out of `looks` the way
# `home.nix`/`scootbar.nix` used to.
#
# Takes only `lib`, like `desktop.nix` itself. Each function takes the
# `programs.scoot` subtree (whatever module reads it: `config` owns it
# on both scoot sides, the bar module reads it across the boundary with
# `or {}` defaults), never the whole config.
{ lib }:

let
  desktop = import ./desktop.nix { inherit lib; };
  # The registry look behind `scootCfg.look`, or null without one (or
  # without a registry entry: a custom `looks` path has no palette
  # here, so the eval-time leaves stay out of its way).
  lookFor =
    scootCfg:
    let
      name = scootCfg.look or null;
    in
    if name == null then null else desktop.looks.${name} or null;
in
{
  inherit lookFor;

  # Whether a theme target applies: a registry look is set, and its
  # `theme.targets.<name>.enable` is on. The `or true` keeps this
  # reading with an older `desktop` subtree that predates the target.
  themed =
    scootCfg: name:
    (lookFor scootCfg) != null && ((scootCfg.desktop.theme.targets.${name}.enable or true));

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

  # The one cursor for every look (Mocu: the same art as the
  # compositor's built-in fallback cursors, built as a real XCursor
  # theme from the vendored SVGs -- see `nix/mocu-cursor-theme.nix` --
  # so compositor-drawn, GTK, Qt, XWayland and greeter cursors all
  # agree). Same name and size everywhere the theme names a cursor:
  # the compositor config, GTK's `settings.ini`,
  # `XCURSOR_THEME`/`XCURSOR_SIZE` and the greeter.
  cursor = {
    name = "Mocu";
    size = 24;
  };

  # The one icon theme for every look (Adwaita, the toolkit default:
  # GTK and Qt both ship expecting it, so nothing is invented).
  iconTheme = "Adwaita";
}
