# The Stylix source of a look (design section 5.3): one pure mapping
# from Stylix's options to a look, used by the catalog derivation and by
# the theme (both read the same result, so the static files and the
# runtime layer never disagree).
#
# Takes only `lib`: no derivations, no import-from-derivation. The one
# function is `synthesize`, over the raw Stylix reads (checked by hand
# against nix-community/stylix at
# `fb28acd59e2ac1984ec84fa496599d6b4bf3e690`, the rev
# `nix/scootbar-tests.nix` names):
#
# - `colors`: `config.lib.stylix.colors.withHashtag` (`base00..base0F`,
#   base16.nix `mkSchemeAttrs`, `stylix/palette.nix`);
# - `fontsSans` / `fontsMono`: `config.stylix.fonts.sansSerif` /
#   `.monospace` (`{ name, package }`, `stylix/fonts.nix`);
# - `sizes`: `config.stylix.fonts.sizes`
#   (`desktop`/`popups`/`applications`/`terminal`, points,
#   `stylix/fonts.nix`);
# - `opacity`: `config.stylix.opacity` (`popups`/`terminal`, 0..1,
#   `stylix/opacity.nix`);
# - `cursor`: `config.stylix.cursor` (`{ name, package, size }` or null,
#   all-or-none by Stylix's own assertion, `stylix/cursor.nix`);
# - `image` / `imageMode`: `config.stylix.image` (a store path by
#   `pathInStore`, or null) and `config.stylix.imageScalingMode` (one of
#   `stretch | fill | fit | center | tile`, exactly scootbg's five modes,
#   `stylix/palette.nix`);
# - `isDark`: the resolved polarity (the caller maps
#   `config.stylix.polarity`: `light`/`dark` verbatim, `either` by the
#   luma rule below);
# - `baseLook`: the registry look behind `extends` (or null without a
#   resolvable base): supplies `appFiles`, `nightTemp` and the fallback
#   faces. A custom-path base has no registry entry, so the static
#   leaves stay inert for it (the runtime layer still works).
#
# Returns `{ json, themeLook, imageFile, fontPackages, hasWallpaper }`:
#
# - `json`: the `scoot-look import-base16` input (a JSON string): the
#   sixteen slots, the companion families/sizes/opacity/cursor, the
#   title and description, and the image file name plus mode when an
#   image is set. Sizes clamp into 4..192 points, opacity into 0..1
#   and cursor size into 4..256 (Stylix's types are wider than the
#   look grammar's; clamping keeps an odd value from failing the
#   build). `polarity` is always explicit here, so the rendered
#   look and `themeLook` below agree by construction.
# - `themeLook`: the registry-shaped look the theme reads for `stylix`
#   (the base with the palette, polarity and faces replaced). No module
#   maps a base16 slot itself: this is the one mapping, and the theme
#   reads tokens, never slots.
# - `imageFile`: the copied image's file name (`wallpaper.<ext>`) or
#   null. The derivation copies the image to it; the JSON names it.
# - `fontPackages`: the Stylix font packages to install (sans and mono,
#   unique, non-null), so the families resolve at runtime. The catalog
#   installs them beside the look.
# - `hasWallpaper`: whether the look carries an image (for the
#   wallpaper package default).
#
# The palette half is the shell-apps plan's table, which checked
# Stylix's own mako and fuzzel targets at `fb28acd` (cards on `base00`,
# borders `base0D`, selection `base02` with `base05` text, match
# `base0A`): `background`/`surface` `base00`, `foreground` `base05`,
# `dim` `base03`, `accent`/`hover`/`focus` `base0D`, `border` `base03`,
# `urgent` `base08`, `accent-fg` `base00`, `selection` `base02`,
# `selection-text` `base05`, `match` `base0A`, terminal in base16
# terminal order (foot `regular0-7` then `bright0-7`: only without
# `extends`, where import-base16 emits `[terminal]`).
{ lib }:

let
  # One hex pair as an integer (exact, no floats): `"ff"` is 255.
  hexVal = {
    "0" = 0;
    "1" = 1;
    "2" = 2;
    "3" = 3;
    "4" = 4;
    "5" = 5;
    "6" = 6;
    "7" = 7;
    "8" = 8;
    "9" = 9;
    a = 10;
    b = 11;
    c = 12;
    d = 13;
    e = 14;
    f = 15;
  };
  digit = c: hexVal.${lib.toLower c} or (throw "programs.scoot.stylix: `${c}` is not a hex digit");
  hexPair = pair: 16 * (digit (builtins.substring 0 1 pair)) + (digit (builtins.substring 1 1 pair));

  # sRGB luma, scaled by 1000 (exact integer math): `0.299 R + 0.587 G
  # + 0.114 B`. An approximation of the WCAG luminance `import-base16`
  # derives polarity from: the two agree on every clearly dark or
  # light scheme, and the Nix side always passes polarity explicitly,
  # so the static files and the rendered look never disagree.
  luma =
    hex:
    let
      bare = lib.removePrefix "#" hex;
    in
    299 * (hexPair (builtins.substring 0 2 bare))
    + 587 * (hexPair (builtins.substring 2 2 bare))
    + 114 * (hexPair (builtins.substring 4 2 bare));

  # Whether `background` reads dark against `foreground`.
  isDarkByLuma = background: foreground: (luma background) <= (luma foreground);

  # The image file name inside the synthesized source: `wallpaper` plus
  # the image's own extension (scootbg sniffs content, but the suffix
  # keeps the source readable). Anything else is a loud eval error.
  imageFileFor =
    image:
    let
      base = baseNameOf (toString image);
      lower = lib.toLower base;
      ext =
        if lib.hasSuffix ".png" lower then
          ".png"
        else if lib.hasSuffix ".jpg" lower then
          ".jpg"
        else if lib.hasSuffix ".jpeg" lower then
          ".jpeg"
        else if lib.hasSuffix ".webp" lower then
          ".webp"
        else if lib.hasSuffix ".gif" lower then
          ".gif"
        else
          throw "programs.scoot.stylix: image `${base}` needs a .png, .jpg, .jpeg, .webp or .gif suffix";
    in
    "wallpaper${ext}";

  synthesize =
    {
      colors,
      fontsSans ? null,
      fontsMono ? null,
      sizes ? { },
      opacity ? { },
      cursor ? null,
      image ? null,
      imageMode ? "fill",
      isDark,
      baseLook ? null,
    }:
    let
      get = slot: colors.${slot} or (throw "programs.scoot.stylix: base16 slot `${slot}` is missing");
      background = get "base00";
      foreground = get "base05";
      dim = get "base03";
      accent = get "base0D";
      urgent = get "base08";
      selection = get "base02";
      match = get "base0A";
      sans = (fontsSans.name or null);
      mono = (fontsMono.name or null);
      ui = sans;
      imageFile = if image == null then null else imageFileFor image;
      fontPackages = lib.unique (
        lib.optional (fontsSans != null && (fontsSans.package or null) != null) fontsSans.package
        ++ lib.optional (fontsMono != null && (fontsMono.package or null) != null) fontsMono.package
      );
      # `import-base16` refuses sizes outside 4..192 points, opacity
      # outside 0..1 and cursor sizes outside 4..256 (the look
      # grammar's ranges); Stylix's own types are wider (any unsigned
      # int or float, any float, any int), so the mapping clamps into
      # range instead of failing the build (the old bar path clamped
      # its pixels the same way).
      clampSize =
        v:
        if v < 4 then
          4
        else if v > 192 then
          192
        else
          v;
      clampOpacity =
        v:
        if v < 0.0 then
          0.0
        else if v > 1.0 then
          1.0
        else
          v;
      clampCursor =
        v:
        if v < 4 then
          4
        else if v > 256 then
          256
        else
          v;
      baseFonts = if baseLook == null then { } else (baseLook.fonts or { });
      json = builtins.toJSON (
        {
          name = "stylix";
          title = "Stylix";
          description = "The Stylix palette as a scoot look.";
          polarity = if isDark then "dark" else "light";
          base00 = background;
          base01 = get "base01";
          base02 = selection;
          base03 = dim;
          base04 = get "base04";
          base05 = foreground;
          base06 = get "base06";
          base07 = get "base07";
          base08 = urgent;
          base09 = get "base09";
          base0A = match;
          base0B = get "base0B";
          base0C = get "base0C";
          base0D = accent;
          base0E = get "base0E";
          base0F = get "base0F";
          fonts = lib.optionalAttrs (sans != null || mono != null) (
            lib.optionalAttrs (sans != null) { sansSerif = sans; }
            // lib.optionalAttrs (mono != null) { monospace = mono; }
          );
          sizes =
            lib.optionalAttrs ((sizes.desktop or null) != null) { desktop = clampSize sizes.desktop; }
            // lib.optionalAttrs ((sizes.popups or null) != null) { popups = clampSize sizes.popups; }
            // lib.optionalAttrs ((sizes.applications or null) != null) {
              applications = clampSize sizes.applications;
            }
            // lib.optionalAttrs ((sizes.terminal or null) != null) { terminal = clampSize sizes.terminal; };
          opacity =
            lib.optionalAttrs ((opacity.popups or null) != null) { popups = clampOpacity opacity.popups; }
            // lib.optionalAttrs ((opacity.terminal or null) != null) {
              terminal = clampOpacity opacity.terminal;
            };
        }
        // lib.optionalAttrs (cursor != null) {
          cursor =
            lib.optionalAttrs ((cursor.name or null) != null) { name = cursor.name; }
            // lib.optionalAttrs ((cursor.size or null) != null) { size = clampCursor cursor.size; };
        }
        // lib.optionalAttrs (imageFile != null) {
          image = imageFile;
          imageScalingMode = imageMode;
        }
      );
      themeLook =
        if baseLook == null then
          {
            appearance = {
              background_color = background;
              focus_ring_active_color = accent;
              focus_ring_inactive_color = dim;
            };
            barColors = {
              inherit
                background
                foreground
                accent
                urgent
                dim
                ;
              hover = accent;
            };
            nightTemp = null;
            inherit isDark;
            fonts = {
              ui = if ui != null then ui else "DejaVu Sans";
              sans = if sans != null then sans else "DejaVu Sans";
              mono = if mono != null then mono else "DejaVu Sans Mono";
            };
            appFiles = {
              foot = null;
              starship = null;
              helixConfig = null;
              helixTheme = null;
              btopConf = null;
              btopTheme = null;
              regreetCss = null;
            };
            wallpaper = null;
          }
        else
          baseLook
          // {
            appearance = {
              background_color = background;
              focus_ring_active_color = accent;
              focus_ring_inactive_color = dim;
            };
            barColors = (baseLook.barColors or { }) // {
              inherit
                background
                foreground
                accent
                urgent
                dim
                ;
              hover = accent;
            };
            inherit isDark;
            fonts = (baseLook.fonts or { }) // {
              ui = if ui != null then ui else baseLook.fonts.ui or "DejaVu Sans";
              sans = if sans != null then sans else baseLook.fonts.sans or "DejaVu Sans";
              mono = if mono != null then mono else baseLook.fonts.mono or "DejaVu Sans Mono";
            };
          };
    in
    {
      inherit
        json
        themeLook
        imageFile
        fontPackages
        ;
      hasWallpaper = image != null;
    };
in
{
  inherit synthesize isDarkByLuma;
}
