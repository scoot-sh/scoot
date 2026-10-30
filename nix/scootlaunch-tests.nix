# Checks for the scootlaunch NixOS/home-manager modules, wired into
# `nix flake check` as `checks.<system>.scootlaunch-modules` (Linux only:
# the launcher does not build elsewhere). Same shape as
# ./scootbar-tests.nix: each module is evaluated standalone with stub
# options standing in for what home-manager and NixOS own, and the
# rendered files are validated by python's tomllib. Then the part no stub
# can stand in for: the REAL scootlaunch binary reads each rendered file
# with `--check` (what an open does before it draws, with no compositor
# to fake) and refuses a file with an unknown key -- the control that
# shows the check can fail.
#
# What this pins:
# - the rendered file holds only the user's `settings` (verbatim, no look
#   layer: the look arrives at runtime under the user's own file);
# - `enable = false` manages nothing; a null package is refused by name;
# - `features` reaches the build as `buildNoDefaultFeatures` +
#   `buildFeatures` and an empty list builds without SVG;
# - the desktop profile (`desktop.launcher.daemon = "scootlaunch"` by
#   default, through a stub of that option subtree) enables this
#   launcher at `mkDefault` (an explicit value still wins), while
#   `daemon = "fuzzel"` leaves it alone;
# - the desktop profile's `theme.overrides.effects` renders into the
#   file's `[effects]` table at `mkDefault` (an explicit `settings` key
#   wins per key); the card and title tables never reach this file;
# - there is no unit and no Stylix switch (the launcher holds nothing
#   when closed; Stylix reaches it only as the `stylix` look);
# - every `--dmenu` flag every Nix call site passes is in scootlaunch's
#   supported set (a new call site adding an unsupported flag fails here,
#   not at a user's keypress);
# - the real binary accepts every rendered file (`scootlaunch --check`,
#   exit 0) and refuses a file with an unknown key (exit 1).
{
  lib,
  pkgs,
  runCommand,
  python3,
  scootlaunch,
  homeModule,
  nixosModule,
}:

let
  tomlFormat = pkgs.formats.toml { };

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
  };

  # The desktop profile as the auto-enable reads it: only the three
  # leaves this module asks of `programs.scoot` (see
  # nix/modules/scootlaunch.nix). The same types and defaults as the
  # real option subtree in nix/modules/desktop.nix, so a default here
  # means the default there.
  profileStub =
    {
      enable ? false,
      launcher ? true,
      daemon ? "scootlaunch",
      # `theme.overrides`: the look-independent effect tokens, empty by
      # default as in the real option.
      overrides ? { },
    }:
    {
      options.programs.scoot.desktop.theme.overrides = lib.genAttrs [ "effects" "card" "title" ] (
        _:
        lib.mkOption {
          type = lib.types.attrsOf lib.types.raw;
          default = { };
        }
      );
      options.programs.scoot.desktop.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.programs.scoot.desktop.launcher.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.programs.scoot.desktop.launcher.daemon = lib.mkOption {
        type = lib.types.enum [
          "fuzzel"
          "scootlaunch"
        ];
        default = "fuzzel";
      };
      config.programs.scoot.desktop = {
        inherit enable;
        theme.overrides = overrides;
        launcher = {
          enable = launcher;
          inherit daemon;
        };
      };
    };

  # A stand-in package that records the arguments `.override` was given.
  fakeLaunch = pkgs.runCommand "fake-scootlaunch" { } ''
    mkdir -p $out/bin
    printf '#!/bin/sh\n' > $out/bin/scootlaunch
    chmod +x $out/bin/scootlaunch
  '';
  recorder = fakeLaunch // {
    override = args: fakeLaunch // { overrideArgs = args; };
  };

  evalWith =
    { side, extra }:
    cfg:
    lib.evalModules {
      modules = [
        (if side == "home" then homeModule else nixosModule)
        baseStubs
        (if side == "home" then homeStubs else nixosStubs)
        {
          programs.scootlaunch = cfg // {
            package = cfg.package or scootlaunch;
          };
        }
      ]
      ++ extra;
      specialArgs = { inherit pkgs; };
    };
  evalHome = evalWith {
    side = "home";
    extra = [ (profileStub { }) ];
  };
  evalHomeProfile =
    stub:
    evalWith {
      side = "home";
      extra = [ (profileStub stub) ];
    };
  evalNixos = evalWith {
    side = "nixos";
    extra = [ (profileStub { }) ];
  };
  evalNixosProfile =
    stub:
    evalWith {
      side = "nixos";
      extra = [ (profileStub stub) ];
    };

  settingsOf = e: e.config.programs.scootlaunch.settings;
  failing = e: map (a: a.message) (lib.filter (a: !a.assertion) e.config.assertions);

  # --- the evaluations ---
  plain = evalHome { enable = true; };
  userWins = evalHome {
    enable = true;
    settings = {
      width_em = 50;
      rows = 12;
      font = "/user/font.ttf";
    };
  };
  oneKey = evalHome {
    enable = true;
    settings.rows = 20;
  };
  recorded = evalHome {
    enable = true;
    package = recorder;
    features = [ "svg" ];
  };
  noSvg = evalHome {
    enable = true;
    package = recorder;
    features = [ ];
  };
  unrecordable = evalHome {
    enable = true;
    package = fakeLaunch;
    features = [ "svg" ];
  };
  off = evalHome { };
  nullPkg = evalHome {
    enable = true;
    package = null;
  };
  os = evalNixos { enable = true; };
  osOff = evalNixos { };
  # The profile default: daemon scootlaunch with the launcher slot on.
  optIn = evalHomeProfile {
    enable = true;
    daemon = "scootlaunch";
  } { };
  # The same, through the stub default (no daemon spelled out).
  defaultOn = evalHomeProfile {
    enable = true;
  } { };
  # The escape hatch: daemon fuzzel leaves this launcher alone.
  optOut = evalHomeProfile {
    enable = true;
    daemon = "fuzzel";
  } { };
  optInDisabled =
    evalHomeProfile
      {
        enable = true;
        daemon = "scootlaunch";
      }
      {
        enable = false;
      };
  optInSlotOff = evalHomeProfile {
    enable = true;
    launcher = false;
    daemon = "scootlaunch";
  } { };
  optInProfileOff = evalHomeProfile { } { };
  osOptIn = evalNixosProfile {
    enable = true;
    daemon = "scootlaunch";
  } { };
  # The profile's look-independent effect tokens: only `[effects]` is
  # the launcher's. The notifier's card and title tables stay out of
  # the file (the launcher refuses a file that carries them).
  overridden = evalHomeProfile {
    enable = true;
    overrides = {
      effects = {
        shadow = "glow";
        motion = "spring";
      };
      card.mark = "stripe";
      title.weight = 600;
    };
  } { };
  overriddenNixos = evalNixosProfile {
    enable = true;
    overrides.effects.fill = "gradient";
  } { };
  overriddenUserWins = evalHomeProfile {
    enable = true;
    overrides.effects = {
      shadow = "glow";
      motion = "spring";
    };
  } { settings.effects.motion = "ease-out"; };

  # Rendered files the real binary checks: every configuration that
  # renders one, plus a hand-made one it must refuse.
  rendered = {
    plain = plain.config.programs.scootlaunch.configFile;
    user-wins = userWins.config.programs.scootlaunch.configFile;
    one-key = oneKey.config.programs.scootlaunch.configFile;
    nixos = os.config.programs.scootlaunch.configFile;
    overridden = overridden.config.programs.scootlaunch.configFile;
    overridden-user-wins = overriddenUserWins.config.programs.scootlaunch.configFile;
  };
  unknownKey = tomlFormat.generate "unknown-key.toml" { colors.backgroun = "#101010"; };

  # Every `--flag` the Nix call sites pass to the menu, read out of the
  # module sources themselves: the keymap's launcher wrapper and
  # clipboard picker (`keys-home.nix`), the screencast chooser
  # (`capture-home.nix`), and the two picker scripts through their menu
  # helper. Only lines that run the menu (its binary variable or a
  # `menu --` call), so unrelated tools' flags never enter. A new call
  # site in a new file must join `menuSources`, or it is unpinned.
  menuSources = [
    ./modules/keys-home.nix
    ./modules/capture-home.nix
    ./modules/nixos.nix
    ./modules/apps-network-pick.sh
    ./modules/apps-bluetooth-pick.sh
  ];
  # scootlaunch's supported set for these call sites (see
  # site/src/content/docs/scootlaunch/cli.md: the dmenu table, the run
  # flag, the layer flag, and the seven per-invocation color flags).
  supportedMenuFlags = [
    "dmenu"
    "prompt"
    "prompt-only"
    "password"
    "no-run-if-empty"
    "only-match"
    "index"
    "placeholder"
    "layer"
    "list-executables-in-path"
    "run"
    "background-color"
    "text-color"
    "border-color"
    "selection-color"
    "selection-text-color"
    "match-color"
    "prompt-color"
  ];
  isMenuLine =
    line:
    builtins.isString line
    && (
      lib.hasInfix "fuzzelPickBin" line
      || lib.hasInfix "menuBin" line
      || lib.hasInfix "launcherBin" line
      || lib.hasInfix "menu --" line
      || lib.hasInfix "\"$MENU\"" line
      # The run bind's flag travels on the bind line beside the wrapper
      # it runs (not a menu invocation line).
      || lib.hasInfix "scoot-launcher" line
    );
  flagsIn =
    line:
    let
      # `builtins.split` returns each capture as a one-entry list
      # (e.g. `"x " ["--dmenu"] " y"`), interleaved with the plain
      # strings around them.
      parts = builtins.split "(--[a-z][a-z-]*)" line;
      caps = builtins.filter builtins.isList parts;
    in
    map (c: lib.removePrefix "--" (builtins.head c)) caps;
  usedMenuFlags = lib.unique (
    lib.concatMap (
      src:
      lib.concatMap flagsIn (builtins.filter isMenuLine (lib.splitString "\n" (builtins.readFile src)))
    ) menuSources
  );

  is = a: b: a == b;
  checks = {
    "empty settings render an empty file, without a look layer" =
      settingsOf plain == { } && !(settingsOf plain ? look);
    "a user's settings render verbatim, and nothing else" =
      settingsOf userWins == {
        width_em = 50;
        rows = 12;
        font = "/user/font.ttf";
      };
    "a user's one key renders beside nothing else" =
      settingsOf oneKey == {
        rows = 20;
      };
    "features reaches the build as no-default-features + the list" =
      recorded.config.programs.scootlaunch.finalPackage.overrideArgs == {
        buildNoDefaultFeatures = true;
        buildFeatures = [ "svg" ];
      };
    "features = [ ] builds without SVG" =
      noSvg.config.programs.scootlaunch.finalPackage.overrideArgs == {
        buildNoDefaultFeatures = true;
        buildFeatures = [ ];
      };
    "features null leaves the package untouched" =
      plain.config.programs.scootlaunch.finalPackage == scootlaunch;
    "features with a package that has no .override is refused by name" =
      lib.any (lib.hasInfix "programs.scootlaunch.features needs") (failing unrecordable);
    "enable = false manages nothing" =
      off.config.home.packages == [ ]
      && off.config.xdg.configFile == { }
      && osOff.config.environment.etc == { };
    "a null package with enable is refused by name" =
      lib.any (lib.hasInfix "programs.scootlaunch.package is null") (failing nullPkg);
    "home: package installed, file at the launcher's default path" =
      plain.config.home.packages == [ scootlaunch ]
      &&
        plain.config.xdg.configFile."scoot/launch.toml".source
        == plain.config.programs.scootlaunch.configFile;
    "nixos: package and an /etc file naming that file" =
      os.config.environment.systemPackages == [ scootlaunch ]
      &&
        os.config.environment.etc."scootlaunch/launch.toml".source
        == os.config.programs.scootlaunch.configFile;
    "no unit: the launcher holds nothing when closed" =
      !(plain.config ? systemd.programs) && plain.config.systemd or { } == { };
    "no Stylix switch: Stylix reaches it only as the stylix look" =
      !(settingsOf plain ? stylix) && !(plain.config.programs.scootlaunch ? stylix);
    "daemon fuzzel leaves this launcher alone" =
      !(optOut.config.programs.scootlaunch.enable or false) && settingsOf optOut == { };
    "the profile default enables this launcher at mkDefault" =
      optIn.config.programs.scootlaunch.enable && defaultOn.config.programs.scootlaunch.enable;
    "an explicit disable still wins over the opt-in" =
      !optInDisabled.config.programs.scootlaunch.enable;
    "the opt-in needs the launcher slot on" =
      !(optInSlotOff.config.programs.scootlaunch.enable or false);
    "the opt-in enables the NixOS half too" = osOptIn.config.programs.scootlaunch.enable;
    "theme.overrides unset renders the file it always did" = settingsOf optIn == { };
    "theme.overrides.effects lands in the file's [effects], and only that table" =
      settingsOf overridden == {
        effects = {
          shadow = "glow";
          motion = "spring";
        };
      };
    "theme.overrides reaches the NixOS file too" =
      settingsOf overriddenNixos == {
        effects.fill = "gradient";
      };
    "an explicit settings key wins over theme.overrides, per key" =
      settingsOf overriddenUserWins == {
        effects = {
          shadow = "glow";
          motion = "ease-out";
        };
      };
    "every menu flag every call site passes is in the supported set" = lib.all (
      f: builtins.elem f supportedMenuFlags
    ) usedMenuFlags;
    "the flag pin really sees the call sites (not an empty list)" =
      lib.all (f: builtins.elem f usedMenuFlags)
        [
          "dmenu"
          "prompt"
          "no-run-if-empty"
          "only-match"
          "layer"
          "list-executables-in-path"
        ];
  };
  failed = lib.attrNames (lib.filterAttrs (_: ok: !ok) checks);

  # Text the shell then compares with what tomllib read back.
  expectations = lib.mapAttrsToList (name: e: {
    inherit name;
    file = e.config.programs.scootlaunch.configFile;
    settings = e.config.programs.scootlaunch.settings;
  }) { inherit plain userWins oneKey; };
in
assert lib.assertMsg (failed == [ ]) "scootlaunch module checks failed: ${builtins.toJSON failed}";
runCommand "scootlaunch-modules"
  {
    nativeBuildInputs = [ python3 ];
    inherit scootlaunch;
    expectations = builtins.toJSON (
      map (x: {
        inherit (x) name settings;
        file = "${x.file}";
      }) expectations
    );
    passAsFile = [ "expectations" ];
    inherit unknownKey;
    files = lib.concatStringsSep " " (lib.mapAttrsToList (n: f: "${n}=${f}") rendered);
    checkNames = builtins.toJSON (lib.attrNames checks);
    usedFlags = builtins.toJSON usedMenuFlags;
  }
  ''
    # 1. Every rendered file parses, and reads back as exactly the
    #    user's settings (verbatim: no look layer, no extra keys).
    python3 - <<'PY'
    import json, os, tomllib
    for x in json.load(open(os.environ["expectationsPath"])):
        got = tomllib.load(open(x["file"], "rb"))
        assert got == x["settings"], (x["name"], got, x["settings"])
        print("toml ok:", x["name"])
    PY

    # 2. The real binary, `--check` over each rendered file (its output
    #    goes in `got`, not `out`, which is $out): what an open does
    #    before it draws, with no compositor to fake. An empty runtime
    #    dir and no `WAYLAND_DISPLAY`: --check touches neither, and the
    #    check would catch it if that changed.
    export XDG_RUNTIME_DIR=$TMPDIR/run HOME=$TMPDIR/home
    mkdir -p "$XDG_RUNTIME_DIR" "$HOME"
    unset WAYLAND_DISPLAY
    for pair in $files; do
      name=''${pair%%=*}; file=''${pair#*=}
      status=0
      $scootlaunch/bin/scootlaunch --check --config "$file" >$TMPDIR/got 2>$TMPDIR/err || status=$?
      if [ "$status" != 0 ]; then
        echo "scootlaunch refused the rendered $name ($file), exit $status:"; cat $TMPDIR/err; cat "$file"; exit 1
      fi
      echo "scootlaunch accepted: $name"
    done
    [ -z "$(ls -A "$XDG_RUNTIME_DIR")" ] || { echo "--check left files in the runtime dir:"; ls -A "$XDG_RUNTIME_DIR"; exit 1; }
    # 3. A file with an unknown key is refused, exit 1: the control
    #    that shows the check can fail.
    status=0
    $scootlaunch/bin/scootlaunch --check --config "$unknownKey" >$TMPDIR/got 2>$TMPDIR/err || status=$?
    if [ "$status" != 1 ]; then
      echo "control failed: the unknown key was not refused with exit 1 (exit $status):"; cat $TMPDIR/err; exit 1
    fi
    echo "scootlaunch refused the unknown key, as it must"

    mkdir -p $out
    echo "$checkNames" > $out/evaluated-checks.json
    echo "$usedFlags" > $out/used-menu-flags.json
  ''
