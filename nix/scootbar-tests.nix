# Checks for the scootbar NixOS/home-manager modules, wired into `nix flake
# check` as `checks.<system>.scootbar-modules` (Linux only: the bar does not
# build elsewhere). Same shape as ./tests.nix: each module is evaluated
# standalone with stub options standing in for what home-manager and NixOS
# own, and the rendered files are validated by python's tomllib. Then the
# part no stub can stand in for: the REAL scootbar binary reads each rendered
# file.
#
# Stylix is stubbed too, and that is what this cannot vouch for: its input is
# not this flake's, so the stub declares just the four things the module
# reads (`lib.stylix.colors.withHashtag.baseNN`, `stylix.enable`,
# `stylix.fonts.sansSerif.{name,package}`, `stylix.fonts.sizes.desktop`),
# checked by hand against nix-community/stylix at fb28acd (stylix/palette.nix,
# stylix/target.nix, stylix/fonts.nix). A rename there would pass this file
# and miss on a real Stylix; the font and color reads are the sites to look at.
#
# What this pins:
# - precedence, highest first: a value the user wrote, then Stylix's, then the
#   module's plain default (a font); per key, so a user's one color keeps
#   Stylix's other four;
# - Stylix is used only when `config.lib.stylix` exists AND `stylix.enable`
#   is on AND `programs.scootbar.stylix.enable` is; otherwise not one Stylix
#   key appears, and the module needs no Stylix option to evaluate;
# - the font is a store FILE path (found in the family's package), never a
#   name, and the size is in pixels;
# - `features` reaches the build as `buildNoDefaultFeatures` +
#   `buildFeatures` and an empty list sets no font;
# - the unit: Restart=on-failure, ordered after and wanted by
#   graphical-session.target, and `systemd.enable = false` writes none;
# - `enable = false` manages nothing; a null package is refused by name;
# - the real binary accepts every rendered file (config, font and module
#   checks all pass, then it stops at "cannot connect to the Wayland
#   compositor", the first thing that needs a compositor) and rejects a file
#   with an unknown key by name -- the control that shows the check can fail.
#   There is no `--check` flag: this is what exists, and it exercises the
#   same `config::load_startup` and font load a real start does.
{
  lib,
  pkgs,
  runCommand,
  python3,
  scootbar,
  homeModule,
  nixosModule,
}:

let
  tomlFormat = pkgs.formats.toml { };

  # A real font, so the real binary can load it: Stylix names one by family
  # and package, as it would for DejaVu Sans.
  dejavu = pkgs.dejavu_fonts.minimal;
  dejavuFile = "${dejavu}/share/fonts/truetype/DejaVuSans.ttf";

  # Only for the file-picking pins (read as a symlink, never loaded).
  distractors = pkgs.runCommand "font-distractors" { } ''
    mkdir -p $out/share/fonts/truetype
    for f in Inter-Bold Inter-Italic Inter_18pt-Regular InterMono-Regular; do
      : > $out/share/fonts/truetype/$f.ttf
    done
    mkdir -p $out/share/fonts/opentype
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

  # The four things the module reads of Stylix, as it defines them.
  stylixStub =
    {
      enable ? true,
      font ? {
        name = "DejaVu Sans";
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
      options.stylix.fonts.sizes.desktop = lib.mkOption { type = lib.types.int; };
      config = {
        stylix.enable = enable;
        stylix.fonts.sansSerif = font;
        stylix.fonts.sizes.desktop = size;
        lib.stylix.colors.withHashtag = {
          base00 = "#101010";
          base03 = "#303030";
          base05 = "#e0e0e0";
          base08 = "#ff0000";
          base0A = "#ffff00";
        };
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
  evalNixos = evalWith {
    side = "nixos";
    extra = [ ];
  };

  settingsOf = e: e.config.programs.scootbar.settings;
  failing = e: map (a: a.message) (lib.filter (a: !a.assertion) e.config.assertions);

  # --- the evaluations ---
  plain = evalHome { enable = true; };
  themed = evalHomeStylix { } { enable = true; };
  # The user's values beat Stylix's, per key.
  userWins = evalHomeStylix { } {
    enable = true;
    settings = {
      colors.background = "#123456";
      bar.font = "/user/font.ttf";
      bar.font-size = 20;
    };
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
  bigSize = evalHomeStylix { size = 200; } { enable = true; };
  zeroSize = evalHomeStylix { size = 0; } { enable = true; };
  pickInter = evalHomeStylix {
    font = {
      name = "Inter";
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
    "no Stylix: no colors, the plain-default font file, no size" =
      settingsOf plain == {
        bar.font = dejavuFile;
      };
    "Stylix on: five colors from base16, the font FILE, the size in pixels" =
      let
        s = settingsOf themed;
      in
      s.colors == {
        background = "#101010";
        foreground = "#e0e0e0";
        accent = "#ffff00";
        dim = "#303030";
        urgent = "#ff0000";
      }
      && s.bar."font-size" == 13
      && builtins.isString s.bar.font
      && s.bar.font != dejavuFile
      && lib.hasPrefix builtins.storeDir s.bar.font;
    "user beats Stylix beats plain: user values kept" =
      let
        s = settingsOf userWins;
      in
      s.colors.background == "#123456" && s.bar.font == "/user/font.ttf" && s.bar."font-size" == 20;
    "user beats Stylix beats plain: the other four colors stay Stylix's" =
      let
        s = settingsOf userWins;
      in
      builtins.removeAttrs s.colors [ "background" ] == {
        foreground = "#e0e0e0";
        accent = "#ffff00";
        dim = "#303030";
        urgent = "#ff0000";
      };
    "a user's one token without Stylix adds only that token" =
      settingsOf profileVsUser == {
        colors.accent = "#abcdef";
        bar.font = dejavuFile;
      };
    "stylix.enable = false (Stylix's own switch) is as if absent" =
      settingsOf stylixDisabled == settingsOf plain;
    "programs.scootbar.stylix.enable = false is as if absent" =
      settingsOf stylixOptOut == settingsOf plain;
    "the size is clamped to the bar's 1..256" =
      (settingsOf bigSize).bar."font-size" == 256 && (settingsOf zeroSize).bar."font-size" == 1;
    "features = [ ] sets no font, with or without Stylix" =
      settingsOf noFont == {
        colors = (settingsOf themed).colors;
        bar."font-size" = 13;
      }
      &&
        (evalHome {
          enable = true;
          features = [ ];
        }).config.programs.scootbar.settings == { };
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
      && u.Service.ExecStart == "${lib.getExe scootbar} daemon"
      && u.Unit.After == [ "graphical-session.target" ]
      && u.Unit.PartOf == [ "graphical-session.target" ]
      && u.Unit.Before == [ "tray.target" ]
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
      && u.serviceConfig.ExecStart == "${lib.getExe scootbar} daemon --config /etc/scootbar/bar.toml"
      && u.after == [ "graphical-session.target" ]
      && u.wantedBy == [ "graphical-session.target" ];
    "nixos: Stylix defaults apply there too" = (settingsOf osThemed).colors.accent == "#ffff00";
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
    # The two font pins read the symlink the module builds.
    pickInter = (settingsOf pickInter).bar.font;
    pickNerd = (settingsOf pickNerd).bar.font;
    themedFont = (settingsOf themed).bar.font;
    inherit dejavuFile distractors;
    files = lib.concatStringsSep " " (lib.mapAttrsToList (n: f: "${n}=${f}") rendered);
    checkNames = builtins.toJSON (lib.attrNames checks);
  }
  ''
    # 1. Every rendered file parses, and reads back as exactly the settings
    #    the module evaluated to.
    python3 - <<'PY'
    import json, os, tomllib
    for x in json.load(open(os.environ["expectationsPath"])):
        got = tomllib.load(open(x["file"], "rb"))
        assert got == x["settings"], (x["name"], got, x["settings"])
        print("toml ok:", x["name"])
    PY

    # 2. The font is found by name inside the family's package.
    [ "$(readlink -f "$themedFont")" = "$dejavuFile" ] || { echo "themed font is not DejaVuSans.ttf: $themedFont"; exit 1; }
    [ "$(readlink -f "$pickInter")" = "$distractors/share/fonts/truetype/Inter_18pt-Regular.ttf" ] || { echo "Inter picked $(readlink -f "$pickInter")"; exit 1; }
    [ "$(readlink -f "$pickNerd")" = "$distractors/share/fonts/opentype/JetBrainsMonoNerdFont-Regular.otf" ] || { echo "Nerd picked $(readlink -f "$pickNerd")"; exit 1; }

    # 3. The real binary. With no compositor, a file it accepts gets past
    #    the config and the font and stops at the connection; a bad one is
    #    refused at the file. `WAYLAND_DISPLAY` unset and an empty runtime
    #    dir, so there is nothing to connect to and no socket to collide.
    export XDG_RUNTIME_DIR=$TMPDIR/run HOME=$TMPDIR/home
    mkdir -p "$XDG_RUNTIME_DIR" "$HOME"
    unset WAYLAND_DISPLAY
    for pair in $files; do
      name=''${pair%%=*}; file=''${pair#*=}
      status=0
      $scootbar/bin/scootbar daemon --config "$file" 2>$TMPDIR/err || status=$?
      if [ "$status" != 1 ] || ! grep -q "cannot connect to the Wayland compositor" $TMPDIR/err; then
        echo "scootbar refused the rendered $name ($file), exit $status:"; cat $TMPDIR/err; cat "$file"; exit 1
      fi
      echo "scootbar accepted: $name"
    done
    status=0
    $scootbar/bin/scootbar daemon --config "$unknownKey" 2>$TMPDIR/err || status=$?
    if [ "$status" != 1 ] || ! grep -q "unknown field .backgroun." $TMPDIR/err; then
      echo "control failed: the unknown key was not refused (exit $status):"; cat $TMPDIR/err; exit 1
    fi
    echo "scootbar refused the unknown key, as it must"

    mkdir -p $out
    echo "$checkNames" > $out/evaluated-checks.json
  ''
