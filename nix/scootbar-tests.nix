# Checks for the scootbar NixOS/home-manager modules, wired into `nix flake
# check` as `checks.<system>.scootbar-modules` (Linux only: the bar does not
# build elsewhere). Same shape as ./tests.nix: each module is evaluated
# standalone with stub options standing in for what home-manager and NixOS
# own, and the rendered files are validated by python's tomllib. Then the
# part no stub can stand in for: the REAL scootbar binary reads each rendered
# file, and the REAL scoot-look binary renders the bar-only Stylix layer.
#
# Stylix is stubbed too, and that is what this cannot vouch for: its input is
# not this flake's, so the stub declares just what the bar-only path reads
# (`config.lib.stylix.colors.withHashtag.baseNN` for all sixteen slots,
# `stylix.enable`, `stylix.fonts.sansSerif` and `.monospace` (`{ name,
# package }`), `stylix.fonts.sizes.desktop/popups/applications/terminal`,
# `stylix.opacity.popups/terminal`, `stylix.cursor`, `stylix.image`,
# `stylix.imageScalingMode`, `stylix.polarity`), checked by hand against
# nix-community/stylix at fb28acd (stylix/palette.nix, stylix/target.nix,
# stylix/fonts.nix, stylix/opacity.nix, stylix/cursor.nix). A rename there
# would pass this file and miss on a real Stylix; the real-Stylix run
# `scripts/scootbar-stylix-test.sh` covers that.
#
# What this pins:
# - precedence, highest first: a value the user wrote, then the look layer
#   at runtime, then the module's plain default (a font); Stylix reaches the
#   file only as the `[look] layer`, never as eval-time values;
# - the bar-only path: with Stylix in use and no scoot catalog, `settings`
#   carry only `[look] layer`, pointing at a fragment whose six colors are
#   the palette, whose `bar.font-size` is Stylix's size in pixels and whose
#   `bar.font` is a store FILE found in the family's package; the
#   plain-default font is left out while the layer covers it;
# - the option does nothing with a catalog present (a warning, no layer),
#   and stays silent without Stylix, opted out, or with no render binary;
# - a scoot module beside the bar without a catalog (desktop off, no
#   scoot-side synthesis) still takes the layer;
# - the font is found by name inside the family's package, through the
#   fragment (Noto and Nerd Font resolve, Inter and Ubuntu fall back with
#   a warning naming the family);
# - `features` reaches the build as `buildNoDefaultFeatures` +
#   `buildFeatures` and an empty list sets no font;
# - the unit: Restart=on-failure, KillMode=process (apps the bar launches
#   survive its restart), ordered after and wanted by
#   graphical-session.target, and `systemd.enable = false` writes none;
# - `enable = false` manages nothing; a null package is refused by name;
# - the real binary accepts every rendered file (`scootbar daemon --check`:
#   the config, the flags over it, the modules and the font, as a start does
#   them, with no compositor to fake) and rejects a file with an unknown key
#   by name -- the control that shows the check can fail.
{
  lib,
  pkgs,
  runCommand,
  python3,
  scootbar,
  scootLook,
  homeModule,
  nixosModule,
}:

let
  tomlFormat = pkgs.formats.toml { };

  # A real font, so the real binaries can load it: Stylix names one by family
  # and package, as it would for DejaVu Sans.
  dejavu = pkgs.dejavu_fonts.minimal;
  dejavuFile = "${dejavu}/share/fonts/truetype/DejaVuSans.ttf";

  # Only for the file-picking pins (read as a symlink, never loaded). The
  # shapes are real packages': noto-fonts ships Family-Regular plus Bold and
  # a Mono family; the Nerd Font one is Family-Regular/Bold in opentype; Inter
  # ships only InterVariable*.ttf and Inter.ttc; Ubuntu ships Ubuntu-R.ttf.
  distractors = pkgs.runCommand "font-distractors" { } ''
    mkdir -p $out/share/fonts/truetype $out/share/fonts/opentype
    for f in NotoSans-Regular NotoSans-Bold NotoSansMono-Regular \
             InterVariable InterVariable-Italic Ubuntu-R Ubuntu-B; do
      : > $out/share/fonts/truetype/$f.ttf
    done
    : > $out/share/fonts/truetype/Inter.ttc
    : > $out/share/fonts/opentype/JetBrainsMonoNerdFont-Bold.otf
    : > $out/share/fonts/opentype/JetBrainsMonoNerdFont-Regular.otf
  '';

  baseStubs = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            assertion = lib.mkOption { type = lib.types.bool; };
            message = lib.mkOption { type = lib.types.str; };
          };
        }
      );
      default = [ ];
    };
    options.warnings = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
    # `config.lib` is an option in both real module systems.
    options.lib = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };

  homeStubs = {
    options.home.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.xdg.configFile = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.source = lib.mkOption { type = lib.types.path; };
        }
      );
      default = { };
    };
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };
  nixosStubs = {
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.environment.etc = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.source = lib.mkOption { type = lib.types.path; };
        }
      );
      default = { };
    };
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };

  # What the bar-only path reads of Stylix, as it defines it: all sixteen
  # base16 slots (each distinct here, so a swapped slot fails the fragment
  # pin), the families, the sizes, the two opacities, and the null cursor,
  # image and either-polarity Stylix itself defaults to.
  stylixStub =
    {
      enable ? true,
      font ? {
        name = "DejaVu Sans";
        package = dejavu;
      },
      monoFont ? {
        name = "DejaVu Sans Mono";
        package = dejavu;
      },
      size ? 10,
    }:
    {
      options.stylix.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.stylix.fonts.sansSerif = lib.mkOption { type = lib.types.raw; };
      options.stylix.fonts.monospace = lib.mkOption { type = lib.types.raw; };
      options.stylix.fonts.sizes = lib.mkOption { type = lib.types.raw; };
      options.stylix.opacity = lib.mkOption { type = lib.types.raw; };
      options.stylix.cursor = lib.mkOption { type = lib.types.raw; };
      options.stylix.image = lib.mkOption { type = lib.types.raw; };
      options.stylix.imageScalingMode = lib.mkOption { type = lib.types.str; };
      options.stylix.polarity = lib.mkOption { type = lib.types.str; };
      config = {
        stylix.enable = enable;
        stylix.fonts.sansSerif = font;
        stylix.fonts.monospace = monoFont;
        stylix.fonts.sizes = {
          desktop = size;
          popups = size;
          applications = 12;
          terminal = 12;
        };
        stylix.opacity = {
          popups = 1.0;
          terminal = 1.0;
        };
        stylix.cursor = null;
        stylix.image = null;
        stylix.imageScalingMode = "fill";
        stylix.polarity = "either";
        lib.stylix.colors.withHashtag = {
          base00 = "#101010";
          base01 = "#111111";
          base02 = "#222222";
          base03 = "#303030";
          base04 = "#444444";
          base05 = "#e0e0e0";
          base06 = "#e6e6e6";
          base07 = "#f0f0f0";
          base08 = "#ff0000";
          base09 = "#ff8800";
          base0A = "#ffff00";
          base0B = "#00ff00";
          base0C = "#00ffff";
          base0D = "#0000ff";
          base0E = "#ff00ff";
          base0F = "#885533";
        };
      };
    };

  # The scoot side as the bar-only path sees it: just the four reads the
  # catalog-presence check uses (`enable`, `desktop.enable`,
  # `stylix.enable`, `lookPackage`), mirroring the catalog-install
  # condition in `nix/modules/home.nix`.
  scootStub =
    {
      enable ? true,
      desktop ? true,
      stylixEnable ? true,
      lookPackage ? fakeBar,
    }:
    {
      options.programs.scoot.enable = lib.mkEnableOption "scoot";
      options.programs.scoot.desktop.enable = lib.mkEnableOption "scoot desktop profile";
      options.programs.scoot.stylix.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
      };
      options.programs.scoot.lookPackage = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
      };
      config.programs.scoot = {
        inherit enable lookPackage;
        desktop.enable = desktop;
        stylix.enable = stylixEnable;
      };
    };

  # A stand-in package that records the arguments `.override` was given.
  fakeBar = pkgs.runCommand "fake-scootbar" { } ''
    mkdir -p $out/bin
    printf '#!/bin/sh\n' > $out/bin/scootbar
    chmod +x $out/bin/scootbar
  '';
  recorder = fakeBar // {
    override = args: fakeBar // { overrideArgs = args; };
  };

  evalWith =
    { side, extra }:
    cfg:
    lib.evalModules {
      modules = [
        (if side == "home" then ../nix/modules/scootbar-home.nix else ../nix/modules/scootbar-nixos.nix)
        baseStubs
        (if side == "home" then homeStubs else nixosStubs)
        {
          programs.scootbar = cfg // {
            package = cfg.package or scootbar;
            lookPackage = cfg.lookPackage or scootLook;
          };
        }
      ]
      ++ extra;
      specialArgs = { inherit pkgs; };
    };
  evalHome = evalWith {
    side = "home";
    extra = [ ];
  };
  evalHomeStylix =
    stub: cfg:
    (evalWith {
      side = "home";
      extra = [ (stylixStub stub) ];
    })
      cfg;
  evalHomeCataloged =
    scoot: cfg:
    (evalWith {
      side = "home";
      extra = [
        (stylixStub { })
        (scootStub scoot)
      ];
    })
      cfg;
  evalNixos = evalWith {
    side = "nixos";
    extra = [ ];
  };

  settingsOf = e: e.config.programs.scootbar.settings;
  warningsOf = e: e.config.warnings;
  layerOf = e: (settingsOf e).look.layer;
  failing = e: map (a: a.message) (lib.filter (a: !a.assertion) e.config.assertions);

  # --- the evaluations ---
  plain = evalHome { enable = true; };
  themed = evalHomeStylix { } { enable = true; };
  # The user's values render as written, beside the same layer.
  userWins = evalHomeStylix { } {
    enable = true;
    settings = {
      colors = {
        background = "#123456";
        foreground = "#234567";
        accent = "#345678";
        hover = "#456789";
        dim = "#56789a";
        urgent = "#6789ab";
      };
      bar.font = "/user/font.ttf";
      bar.font-size = 20;
    };
  };
  oneToken = evalHomeStylix { } {
    enable = true;
    settings.colors.accent = "#abcdef";
  };
  # The flake's own module system: the user's `lib.mkForce`-free plain value
  # against a `mkDefault` set by another module (a profile, say) is the
  # same rule one level up.
  profileVsUser = evalHome {
    enable = true;
    settings.colors.accent = "#abcdef";
  };
  stylixDisabled = evalHomeStylix { enable = false; } { enable = true; };
  stylixOptOut = evalHomeStylix { } {
    enable = true;
    stylix.enable = false;
  };
  noLookPackage = evalHomeStylix { } {
    enable = true;
    lookPackage = null;
  };
  # A scoot catalog present: the desktop profile with a render binary.
  cataloged = evalHomeCataloged { } { enable = true; };
  # Scoot beside the bar but no catalog: the desktop off without
  # scoot-side synthesis.
  standaloneWithScoot =
    evalHomeCataloged
      {
        desktop = false;
        stylixEnable = false;
      }
      {
        enable = true;
      };
  # The desktop on but no render binary there: no catalog installs, so the
  # bar-only layer still applies.
  filesOnlyScoot = evalHomeCataloged { lookPackage = null; } {
    enable = true;
  };
  bigSize = evalHomeStylix { size = 200; } { enable = true; };
  zeroSize = evalHomeStylix { size = 0; } { enable = true; };
  pickNoto = evalHomeStylix {
    font = {
      name = "Noto Sans";
      package = distractors;
    };
  } { enable = true; };
  # No regular-face file by these names: the fallback face, with a warning.
  pickInter = evalHomeStylix {
    font = {
      name = "Inter";
      package = distractors;
    };
  } { enable = true; };
  pickUbuntu = evalHomeStylix {
    font = {
      name = "Ubuntu";
      package = distractors;
    };
  } { enable = true; };
  pickNerd = evalHomeStylix {
    font = {
      name = "JetBrainsMono Nerd Font";
      package = distractors;
    };
  } { enable = true; };
  noFont = evalHomeStylix { } {
    enable = true;
    features = [ ];
  };
  noFontPlain = evalHome {
    enable = true;
    features = [ ];
  };
  recorded = evalHome {
    enable = true;
    package = recorder;
    features = [
      "clock"
      "workspaces"
    ];
  };
  unrecordable = evalHome {
    enable = true;
    package = fakeBar;
    features = [ "clock" ];
  };
  off = evalHome { };
  nullPkg = evalHome {
    enable = true;
    package = null;
  };
  noUnit = evalHome {
    enable = true;
    systemd.enable = false;
  };
  os = evalNixos { enable = true; };
  osThemed = evalWith {
    side = "nixos";
    extra = [ (stylixStub { }) ];
  } { enable = true; };
  osOff = evalNixos { };

  # Rendered files the real binary reads: every configuration that
  # renders a file, plus a hand-made one it must refuse.
  rendered = {
    plain = plain.config.programs.scootbar.configFile;
    themed = themed.config.programs.scootbar.configFile;
    user-wins-except-font =
      (evalHomeStylix { } {
        enable = true;
        settings = {
          colors.background = "#123456";
          bar.font-size = 20;
        };
      }).config.programs.scootbar.configFile;
    full =
      (evalHomeStylix { } {
        enable = true;
        settings = {
          left = [ "clock" ];
          center = [ ];
          bar = {
            height = 32;
            opacity = 0.9;
            margin = "8,4";
          };
          clock.format = "%H:%M";
          "output"."eDP-1".height = 36;
        };
      }).config.programs.scootbar.configFile;
    nixos = os.config.programs.scootbar.configFile;
  };
  unknownKey = tomlFormat.generate "unknown-key.toml" { colors.backgroun = "#101010"; };

  is = a: b: a == b;
  checks = {
    "no Stylix: the plain-default font file and nothing else, silently" =
      settingsOf plain == {
        bar.font = dejavuFile;
      }
      && warningsOf plain == [ ];
    "Stylix on, no catalog: settings carry only [look] layer, silently" =
      let
        s = settingsOf themed;
      in
      builtins.attrNames s == [ "look" ]
      && builtins.attrNames s.look == [ "layer" ]
      && lib.hasPrefix builtins.storeDir s.look.layer
      && lib.hasSuffix "/bar.toml" s.look.layer
      && warningsOf themed == [ ];
    "a user's values render as written beside the same layer" =
      let
        s = settingsOf userWins;
      in
      s.colors == {
        background = "#123456";
        foreground = "#234567";
        accent = "#345678";
        hover = "#456789";
        dim = "#56789a";
        urgent = "#6789ab";
      }
      && s.bar.font == "/user/font.ttf"
      && s.bar."font-size" == 20
      && s.look.layer == (settingsOf themed).look.layer;
    "a user's one token renders beside the layer, and nothing else" =
      settingsOf oneToken == {
        colors.accent = "#abcdef";
        look.layer = (settingsOf themed).look.layer;
      };
    "a user's one token without Stylix adds only that token" =
      settingsOf profileVsUser == {
        colors.accent = "#abcdef";
        bar.font = dejavuFile;
      };
    "stylix.enable = false (Stylix's own switch) is as if absent" =
      settingsOf stylixDisabled == settingsOf plain && warningsOf stylixDisabled == [ ];
    "programs.scootbar.stylix.enable = false is as if absent" =
      settingsOf stylixOptOut == settingsOf plain && warningsOf stylixOptOut == [ ];
    "no render binary is as if absent: the plain default returns, silently" =
      settingsOf noLookPackage == settingsOf plain && warningsOf noLookPackage == [ ];
    "with a scoot catalog present the option warns and sets no layer" =
      !(settingsOf cataloged ? look)
      && lib.any (lib.hasInfix "programs.scootbar.stylix.enable has no effect") (warningsOf cataloged);
    "scoot beside the bar without a catalog still takes the same layer, silently" =
      layerOf standaloneWithScoot == layerOf themed && warningsOf standaloneWithScoot == [ ];
    "the desktop on but no render binary there installs no catalog, so the layer stays" =
      layerOf filesOnlyScoot == layerOf themed && warningsOf filesOnlyScoot == [ ];
    "a large and a zero size still render a layer (clamped in the fragment)" =
      lib.hasSuffix "/bar.toml" (layerOf bigSize) && lib.hasSuffix "/bar.toml" (layerOf zeroSize);
    "every picked family still renders a layer" = lib.all (e: lib.hasSuffix "/bar.toml" (layerOf e)) [
      pickNoto
      pickInter
      pickUbuntu
      pickNerd
    ];
    "features = [ ] sets no font: only the layer with Stylix, nothing without" =
      settingsOf noFont == {
        look.layer = (settingsOf themed).look.layer;
      }
      && settingsOf noFontPlain == { };
    "features reaches the build as no-default-features + the list" =
      recorded.config.programs.scootbar.finalPackage.overrideArgs == {
        buildNoDefaultFeatures = true;
        buildFeatures = [
          "clock"
          "workspaces"
        ];
      };
    "features null leaves the package untouched" =
      plain.config.programs.scootbar.finalPackage == scootbar;
    "features with a package that has no .override is refused by name" =
      lib.any (lib.hasInfix "programs.scootbar.features needs") (failing unrecordable);
    "enable = false manages nothing" =
      off.config.home.packages == [ ]
      && off.config.xdg.configFile == { }
      && off.config.systemd.user.services == { }
      && osOff.config.environment.etc == { }
      && osOff.config.systemd.user.services == { };
    "a null package with enable is refused by name" =
      lib.any (lib.hasInfix "programs.scootbar.package is null") (failing nullPkg);
    "home: package installed, file at the bar's default path" =
      plain.config.home.packages == [ scootbar ]
      && plain.config.xdg.configFile."scoot/bar.toml".source == plain.config.programs.scootbar.configFile;
    "home: the unit restarts on failure, after and wanted by the graphical session" =
      let
        u = plain.config.systemd.user.services.scootbar;
      in
      u.Service.Restart == "on-failure"
      && u.Service.KillMode == "process"
      && u.Service.ExecStart == "${lib.getExe scootbar} daemon"
      && u.Unit.After == [ "graphical-session.target" ]
      && u.Unit.PartOf == [ "graphical-session.target" ]
      && u.Unit.Before == [ "tray.target" ]
      && u.Unit.StartLimitIntervalSec == 0
      && u.Install.WantedBy == [ "graphical-session.target" ]
      && u.Unit.X-Restart-Triggers == [ "${plain.config.programs.scootbar.configFile}" ];
    "home: systemd.enable = false writes no unit and keeps the file" =
      noUnit.config.systemd.user.services == { } && noUnit.config.xdg.configFile ? "scoot/bar.toml";
    "nixos: package, /etc file, and a unit naming that file" =
      let
        u = os.config.systemd.user.services.scootbar;
      in
      os.config.environment.systemPackages == [ scootbar ]
      && os.config.environment.etc."scootbar/bar.toml".source == os.config.programs.scootbar.configFile
      && u.serviceConfig.Restart == "on-failure"
      && u.serviceConfig.KillMode == "process"
      && u.serviceConfig.ExecStart == "${lib.getExe scootbar} daemon --config /etc/scootbar/bar.toml"
      && u.after == [ "graphical-session.target" ]
      && u.unitConfig.StartLimitIntervalSec == 0
      && u.wantedBy == [ "graphical-session.target" ];
    "nixos: the bar-only layer applies there too, the same fragment" =
      layerOf osThemed == layerOf themed;
  };
  failed = lib.attrNames (lib.filterAttrs (_: ok: !ok) checks);

  # Text the shell then compares with what tomllib read back.
  expectations = lib.mapAttrsToList (name: e: {
    inherit name;
    file = e.config.programs.scootbar.configFile;
    settings = e.config.programs.scootbar.settings;
  }) { inherit plain themed userWins; };
in
assert lib.assertMsg (failed == [ ]) "scootbar module checks failed: ${builtins.toJSON failed}";
runCommand "scootbar-modules"
  {
    nativeBuildInputs = [ python3 ];
    inherit scootbar;
    expectations = builtins.toJSON (
      map (x: {
        inherit (x) name settings;
        file = "${x.file}";
      }) expectations
    );
    passAsFile = [ "expectations" ];
    inherit unknownKey;
    # The bar-only layers whose fragments the pins below read: each build
    # runs the real `scoot-look` over the synthesized Stylix reads.
    themedLayer = layerOf themed;
    bigLayer = layerOf bigSize;
    zeroLayer = layerOf zeroSize;
    notoLayer = layerOf pickNoto;
    interLayer = layerOf pickInter;
    ubuntuLayer = layerOf pickUbuntu;
    nerdLayer = layerOf pickNerd;
    inherit dejavuFile distractors;
    files = lib.concatStringsSep " " (lib.mapAttrsToList (n: f: "${n}=${f}") rendered);
    checkNames = builtins.toJSON (lib.attrNames checks);
  }
  ''
    # 1. Every rendered file parses, and reads back as exactly the settings
    #    the module evaluated to (user values verbatim, the layer under them).
    python3 - <<'PY'
    import json, os, tomllib
    for x in json.load(open(os.environ["expectationsPath"])):
        got = tomllib.load(open(x["file"], "rb"))
        assert got == x["settings"], (x["name"], got, x["settings"])
        print("toml ok:", x["name"])
    PY

    # 2. The bar-only layer: the fragment each themed file points at carries
    #    the palette (background base00, foreground base05, accent and hover
    #    base0D like the compositor's focused ring, dim base03, urgent base08)
    #    and the size in pixels (the bar clamps 1..256).
    python3 - "$themedLayer" "$bigLayer" "$zeroLayer" <<'PY'
    import sys, tomllib
    themed, big, zero = (tomllib.load(open(p, "rb")) for p in sys.argv[1:4])
    assert themed["colors"] == {
        "background": "#101010",
        "foreground": "#e0e0e0",
        "accent": "#0000ff",
        "hover": "#0000ff",
        "dim": "#303030",
        "urgent": "#ff0000",
    }, themed["colors"]
    assert len(set(themed["colors"].values())) == 5, (
        "two tokens share a color beyond hover==accent, so a swapped slot would pass",
        themed["colors"],
    )
    assert themed["bar"]["font-size"] == 13, themed["bar"]
    assert big["bar"]["font-size"] == 256, big["bar"]
    assert zero["bar"]["font-size"] == 1, zero["bar"]
    for name, frag in (("themed", themed), ("big", big), ("zero", zero)):
        font = frag["bar"]["font"]
        assert font.startswith("/nix/store/") and font.endswith((".ttf", ".otf")), (name, font)
        print("layer ok:", name, "size", frag["bar"]["font-size"], "font", font)
    PY

    # 3. The font is found by family inside its package, through the
    #    fragment. No regular face by that name (variable font, .ttc,
    #    Ubuntu-R): the fallback face, and the derivation's warning names
    #    the family.
    fontOf() { python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["bar"]["font"])' "$1"; }
    [ "$(readlink -f "$(fontOf "$themedLayer")")" = "$dejavuFile" ] || { echo "themed font is not DejaVuSans.ttf: $(fontOf "$themedLayer")"; exit 1; }
    [ "$(readlink -f "$(fontOf "$notoLayer")")" = "$distractors/share/fonts/truetype/NotoSans-Regular.ttf" ] || { echo "Noto picked $(readlink -f "$(fontOf "$notoLayer")")"; exit 1; }
    for pair in "Inter=$interLayer" "Ubuntu=$ubuntuLayer"; do
      fam=''${pair%%=*}; layer=''${pair#*=}; font=$(fontOf "$layer")
      [ "$(readlink -f "$font")" = "$dejavuFile" ] || { echo "$fam did not fall back to DejaVu Sans: $(readlink -f "$font")"; exit 1; }
      grep -q "no regular-face file named for \"$fam\"" "$(dirname "$font")/warning" || { echo "no warning for $fam:"; cat "$(dirname "$font")/warning"; exit 1; }
    done
    # A resolved font leaves the warning empty.
    [ ! -s "$(dirname "$(fontOf "$notoLayer")")/warning" ] || { echo "warning on a resolved font"; exit 1; }
    [ "$(readlink -f "$(fontOf "$nerdLayer")")" = "$distractors/share/fonts/opentype/JetBrainsMonoNerdFont-Regular.otf" ] || { echo "Nerd picked $(readlink -f "$(fontOf "$nerdLayer")")"; exit 1; }

    # 4. The real binary, `daemon --check` (its output goes in `got`, not
    #    `out`, which is $out): what a start does before it
    #    connects (the file, the font, the modules, the layer merge), with
    #    nothing to connect to. A bad file is refused at the file, exit 1.
    #    An empty runtime dir and no `WAYLAND_DISPLAY`: --check touches
    #    neither, and the check would catch it if that changed.
    export XDG_RUNTIME_DIR=$TMPDIR/run HOME=$TMPDIR/home
    mkdir -p "$XDG_RUNTIME_DIR" "$HOME"
    unset WAYLAND_DISPLAY
    for pair in $files; do
      name=''${pair%%=*}; file=''${pair#*=}
      status=0
      got=$($scootbar/bin/scootbar daemon --check --config "$file" 2>$TMPDIR/err) || status=$?
      if [ "$status" != 0 ] || [ "$got" != ok ] || [ -s $TMPDIR/err ]; then
        echo "scootbar refused the rendered $name ($file), exit $status:"; cat $TMPDIR/err; cat "$file"; exit 1
      fi
      echo "scootbar accepted: $name"
    done
    [ -z "$(ls -A "$XDG_RUNTIME_DIR")" ] || { echo "--check left files in the runtime dir:"; ls -A "$XDG_RUNTIME_DIR"; exit 1; }
    status=0
    $scootbar/bin/scootbar daemon --check --config "$unknownKey" 2>$TMPDIR/err || status=$?
    if [ "$status" != 1 ] || ! grep -q "unknown field .backgroun." $TMPDIR/err; then
      echo "control failed: the unknown key was not refused (exit $status):"; cat $TMPDIR/err; exit 1
    fi
    echo "scootbar refused the unknown key, as it must"

    mkdir -p $out
    echo "$checkNames" > $out/evaluated-checks.json
  ''
