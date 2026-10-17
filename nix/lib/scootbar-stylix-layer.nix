# The bar-only Stylix path (design section 5.5): a user who runs scootbar
# without scoot has no look catalog, so the bar module renders the
# `stylix` look's bar fragment itself and pins it as a fixed `[look]
# layer` in the rendered `bar.toml`.
#
# A function over `{ config, cfg, scootLook, fallbackFont }`, returning
# `{ active, catalogPresent, layer, warning }`. `nix/modules/scootbar.nix`
# calls it and reads only the result, so no app module reads Stylix
# itself: the reads below are the bar half's one place, beside the
# scoot-side synthesis (`nix/lib/look-from-stylix.nix`, whose
# `synthesize` this reuses — the slot mapping is not duplicated here).
#
# Every Stylix read checked by hand against nix-community/stylix at
# `fb28acd59e2ac1984ec84fa496599d6b4bf3e690` (the rev
# `nix/scootbar-tests.nix` names):
#
# - `config.lib.stylix.colors.withHashtag` (`base00..base0F`, base16.nix
#   `mkSchemeAttrs`, `stylix/palette.nix`);
# - `config.stylix.enable` (`stylix/target.nix`);
# - `config.stylix.fonts.sansSerif` / `.monospace` (`{ name, package }`,
#   `stylix/fonts.nix`);
# - `config.stylix.fonts.sizes` (`desktop`/`popups`/`applications`/
#   `terminal`, points, `stylix/fonts.nix`);
# - `config.stylix.opacity` (`popups`/`terminal`, 0..1,
#   `stylix/opacity.nix`);
# - `config.stylix.cursor` (`{ name, package, size }` or null, all-or-none
#   by Stylix's own assertion, `stylix/cursor.nix`);
# - `config.stylix.image` (a store path by `pathInStore`, or null) and
#   `config.stylix.imageScalingMode` (one of `stretch | fill | fit |
#   center | tile`, `stylix/palette.nix`);
# - `config.stylix.polarity` (`either`/`light`/`dark`,
#   `stylix/palette.nix`).
#
# `active` holds only when all of these do: the bar is enabled, its
# `stylix.enable` is on, Stylix itself is in use, no scoot look catalog
# is installed by this configuration, and `scootLook` (the `scoot-look`
# binary package) is on hand. The catalog check mirrors the
# catalog-install condition in `nix/modules/home.nix` (the NixOS side
# installs under the same condition, minus the synthesis half): the
# desktop profile, or the scoot-side Stylix synthesis (which carries its
# own look), with a render binary there. Each `or` keeps this evaluating
# with the scoot module absent.
#
# The fragment is built like the catalog's synthesized `stylix` look
# (`nix/look-catalog.nix` `mkStylix` and the `styRendered` build:
# `scoot-look import-base16` over the serialized reads, then `scoot-look
# build --name stylix --origin declared --catalog`), but with no
# `extends`: the bar-only look has no base. The font file follows the
# catalog's `mkFontFile` rule (the regular face whose file name is the
# family name, or that name plus `-Regular`, else the fallback face with
# a build-time warning). `layer` is the rendered `bar.toml`; it is only
# evaluated when `active` holds, so nothing here touches Stylix (or the
# render binary) otherwise.
{ lib, pkgs }:

{
  config,
  cfg,
  scootLook,
  fallbackFont,
}:

let
  # Stylix is not an input of this flake: its presence is
  # `config.lib.stylix` (defined by its palette module whether or not it
  # is enabled), and `stylix.enable` (false by default in Stylix itself)
  # says whether it is asked to theme anything. `&&` short-circuits, so
  # `config.stylix` is never touched without the module.
  stylixPresent = config.lib ? stylix && (config.stylix.enable or false);

  # The scoot side, when its module is imported beside this one. Each
  # `or` keeps this evaluating with it absent: nothing here requires it.
  scootCfg = (config.programs.scoot or { });
  scootDesktopOn = ((scootCfg.desktop or { }).enable or false);
  scootStylixOn = ((scootCfg.stylix or { }).enable or false);
  scootLookPkg = scootCfg.lookPackage or null;
  scootStylixActive =
    (scootCfg.enable or false) && scootStylixOn && stylixPresent && scootLookPkg != null;
  catalogPresent = (scootDesktopOn || scootStylixActive) && scootLookPkg != null;

  # The bar-only path, active only when all hold.
  active = cfg.enable && cfg.stylix.enable && stylixPresent && !catalogPresent && scootLook != null;

  # With a catalog present the option does nothing, and says so: the bar
  # follows the active look through `scoot look sync` instead of a fixed
  # layer. The module surfaces this in `warnings` (pinnable in
  # `nix/scootbar-tests.nix`); the message names the opt-out.
  warning =
    if cfg.enable && cfg.stylix.enable && stylixPresent && catalogPresent then
      "programs.scootbar.stylix.enable has no effect: a scoot look catalog is present, so the bar follows the active look instead of a fixed Stylix layer. Set programs.scootbar.stylix.enable = false to silence this."
    else
      null;

  stylixLib = import ./look-from-stylix.nix { inherit lib; };

  colors = config.lib.stylix.colors.withHashtag;
  fontsCfg = (config.stylix.fonts or { });
  sizesCfg = (fontsCfg.sizes or { });
  opacityCfg = (config.stylix.opacity or { });
  # `light`/`dark` verbatim, `either` (Stylix's default: automatic) by
  # the luma rule — the same mapping `nix/modules/home.nix` uses, so the
  # fragment and the scoot-side synthesis agree.
  isDark =
    let
      polarity = config.stylix.polarity or "either";
    in
    if polarity == "dark" then
      true
    else if polarity == "light" then
      false
    else
      stylixLib.isDarkByLuma colors.base00 colors.base05;

  # The `scoot-look import-base16` input, through the one mapping (no
  # base look: the bar-only look extends nothing).
  synth = stylixLib.synthesize {
    inherit colors isDark;
    fontsSans = fontsCfg.sansSerif or null;
    fontsMono = fontsCfg.monospace or null;
    sizes = {
      desktop = sizesCfg.desktop or null;
      popups = sizesCfg.popups or null;
      applications = sizesCfg.applications or null;
      terminal = sizesCfg.terminal or null;
    };
    opacity = {
      popups = opacityCfg.popups or null;
      terminal = opacityCfg.terminal or null;
    };
    cursor = config.stylix.cursor or null;
    image = config.stylix.image or null;
    imageMode = config.stylix.imageScalingMode or "fill";
    baseLook = null;
  };
  jsonFile = pkgs.writeText "scootbar-stylix-base16.json" synth.json;

  src = pkgs.runCommand "scootbar-stylix-src" { nativeBuildInputs = [ scootLook ]; } ''
    mkdir -p $out/stylix
    scoot-look import-base16 "${jsonFile}" --name stylix --out $out/stylix
    ${lib.optionalString (
      synth.imageFile != null
    ) "cp \"${config.stylix.image}\" \"$out/stylix/${synth.imageFile}\""}
  '';

  # The one-entry catalog the build resolves against.
  sources = pkgs.runCommand "scootbar-stylix-sources" { } ''
    mkdir -p $out
    ln -s "${src}/stylix" $out/stylix
  '';

  # The bar's UI face as a file, found at build time in the carrier
  # package's `share/fonts`: the same rule the catalog's `mkFontFile`
  # uses, so the two never disagree. Without a family or a package the
  # render carries no `--ui-font-file` and the runtime resolves the
  # family through fontconfig.
  sans = fontsCfg.sansSerif or null;
  fontFamily = if sans == null then null else (sans.name or null);
  fontPackage = if sans == null then null else (sans.package or null);
  fontFile =
    if fontFamily == null || fontPackage == null then
      null
    else
      pkgs.runCommand "scootbar-stylix-font"
        {
          nativeBuildInputs = [ pkgs.findutils ];
          family = fontFamily;
          package = fontPackage;
          fallback = fallbackFont;
        }
        ''
          norm() { tr -d ' _-' | tr '[:upper:]' '[:lower:]'; }
          want=$(printf '%s' "$family" | norm)
          files=$(find -L "$package/share/fonts" -type f \( -iname '*.ttf' -o -iname '*.otf' \) 2>/dev/null | sort || true)
          pick=
          while IFS= read -r file; do
            [ -n "$file" ] || continue
            base=$(basename "$file"); base=''${base%.*}
            have=$(printf '%s' "$base" | norm)
            if [ "$have" = "$want" ] || [ "$have" = "''${want}regular" ]; then
              pick=$file
              break
            fi
          done <<<"$files"
          mkdir $out
          : > $out/warning
          if [ -z "$pick" ]; then
            msg="scootbar: no regular-face file named for \"$family\" in $package/share/fonts (variable fonts, .ttc and other naming are not resolved); using the fallback face. Set programs.scootbar.settings.bar.font to a .ttf/.otf path to choose."
            echo "warning: $msg" >&2
            echo "$msg" > $out/warning
            pick=$fallback
          fi
          ln -s "$pick" $out/font
        '';

  wallpaperAbs = if synth.imageFile == null then null else "${src}/stylix/${synth.imageFile}";

  rendered =
    pkgs.runCommand "scootbar-stylix-rendered"
      {
        nativeBuildInputs = [ scootLook ];
      }
      ''
        mkdir -p $out
        scoot-look build "${src}/stylix" --out "$out" --name stylix --origin declared --catalog "${sources}" \
          ${lib.optionalString (wallpaperAbs != null) "--wallpaper-abs \"${wallpaperAbs}\""} \
          ${lib.optionalString (fontFile != null) "--ui-font-file \"${fontFile}/font\""}
      '';

  layer = "${rendered}/bar.toml";
in
{
  inherit
    active
    catalogPresent
    layer
    warning
    ;
}
