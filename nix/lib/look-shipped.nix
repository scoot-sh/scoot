# The shipped looks as evaluation data, read from the look directories
# (`looks/<name>/look.toml`) instead of a hand-maintained registry, plus
# the one helper every desktop slot reads look-derived values through
# (never hand-mapped per target out of `looks` the way `home.nix` and
# `scootbar.nix` used to). Replaces the `desktop.nix` registry and
# `theme-look.nix`: the directories are the single source of truth, so no
# equality check keeps two copies in step.
#
# Takes only `lib`, like `desktop.nix` itself. Each function takes the
# `programs.scoot` subtree (whatever module reads it: `config` owns it
# on both scoot sides, the bar module reads it across the boundary with
# `or {}` defaults), never the whole config.
#
# A shipped look's record keeps the registry's shape (what every leaf
# below reads):
# - `appearance` (the 3 compositor colors), `barColors` (the 6 roles;
#   `hover` is absent where the manifest names none, and readers fall
#   back to the accent), `nightTemp`, `isDark` (from `polarity`),
#   `fonts` (`ui`/`sans`/`mono`);
# - `appFiles` (the example's own static files, hand-tuned per look,
#   not derivable from the roles without inventing colors): present
#   where the example ships the file, else null;
# - `wallpaper`: null for a color-only look, else the image, its mode
#   and its fill where the manifest names one. The image resolves by
#   convention, in order: `docs/assets/wallpapers/<name>.png` (the
#   shared illustration the session themes from), then
#   `docs/examples/<name>/wallpaper.png`, else null.
{ lib }:

let
  lookDir = ../../looks;
  exampleDir = ../../docs/examples;
  assetsDir = ../../docs/assets/wallpapers;

  # Every directory under `looks/` carrying a manifest, sorted (like
  # every directory scan in the catalog: order among declared entries
  # is the catalog's own, and the runtime scan sorts within a
  # directory).
  shippedNames = builtins.attrNames (
    lib.filterAttrs (
      name: kind: kind == "directory" && builtins.pathExists (lookDir + "/${name}/look.toml")
    ) (builtins.readDir lookDir)
  );

  # One example file, or null where the example ships none.
  exampleFile =
    name: rel:
    if builtins.pathExists (exampleDir + "/${name}/${rel}") then
      exampleDir + "/${name}/${rel}"
    else
      null;

  # The one file with `suffix` directly under the example's `relDir`,
  # or null (missing directory, none, or more than one).
  oneExampleFile =
    name: relDir: suffix:
    let
      dir = exampleDir + "/${name}/${relDir}";
      hits =
        if builtins.pathExists dir then
          builtins.attrNames (
            lib.filterAttrs (entry: kind: kind == "regular" && lib.hasSuffix suffix entry) (
              builtins.readDir dir
            )
          )
        else
          [ ];
    in
    if builtins.length hits == 1 then dir + "/${builtins.head hits}" else null;

  toLook =
    name:
    let
      t = builtins.fromTOML (builtins.readFile (lookDir + "/${name}/look.toml"));
      wall = t.wallpaper or { };
      fill = wall.fill or null;
      image =
        if builtins.pathExists (assetsDir + "/${name}.png") then
          assetsDir + "/${name}.png"
        else if builtins.pathExists (exampleDir + "/${name}/wallpaper.png") then
          exampleDir + "/${name}/wallpaper.png"
        else
          null;
      helixConfig = exampleFile name "helix/config.toml";
      btopNested = exampleFile name "btop/btop.conf";
    in
    {
      appearance = {
        background_color = t.palette.background;
        focus_ring_active_color = t.palette.focus;
        focus_ring_inactive_color = t.palette.border;
      };
      barColors = {
        background = t.palette.background;
        foreground = t.palette.foreground;
        accent = t.palette.accent;
        dim = t.palette.dim;
        urgent = t.palette.urgent;
      }
      // lib.optionalAttrs (t.palette ? hover) { hover = t.palette.hover; };
      nightTemp = t.night.temperature;
      isDark = t.polarity == "dark";
      fonts = {
        ui = t.fonts.ui;
        sans = t.fonts.sans;
        mono = t.fonts.mono;
      };
      appFiles = {
        foot = exampleFile name "foot.ini";
        starship = exampleFile name "starship.toml";
        helixConfig = helixConfig;
        helixTheme = if helixConfig == null then null else oneExampleFile name "helix/themes" ".toml";
        btopConf = if btopNested != null then btopNested else exampleFile name "btop.conf";
        btopTheme = oneExampleFile name "btop/themes" ".theme";
        regreetCss = exampleFile name "regreet.css";
      };
      wallpaper =
        if image == null then
          null
        else
          {
            inherit image;
            mode = wall.mode;
          }
          // lib.optionalAttrs (fill != null) { inherit fill; };
    };

  looks = lib.listToAttrs (
    map (name: {
      inherit name;
      value = toLook name;
    }) shippedNames
  );

  # The shipped look behind `scootCfg.look`, or null without one (or
  # without a shipped entry: a custom `looks` path has no palette
  # here, so the eval-time leaves stay out of its way). `stylix`
  # resolves through the synthesized theme
  # (`programs.scoot.stylix.theme`, set beside the catalog when Stylix
  # synthesis is on): the Stylix palette with the base look's static
  # leaves, so the eval-time files move with the active look.
  lookFor =
    scootCfg:
    let
      name = scootCfg.look or null;
    in
    if name == null then
      null
    else if name == "stylix" then
      scootCfg.stylix.theme or null
    else
      looks.${name} or null;
in
{
  inherit shippedNames looks lookFor;

  # Whether a theme target applies: a look is set (a shipped look or
  # the synthesized `stylix` one), and its
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
