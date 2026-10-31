# The part of `programs.scootlaunch` that home-manager and NixOS share:
# the options, the rendered `launch.toml` and the package with its Cargo
# features. `scootlaunch-home.nix` and `scootlaunch-nixos.nix` import it
# and add only what their side has (where the file goes). Written for
# site/src/content/docs/scootlaunch/index.md ("Overview").
#
# The precedence of a `settings` value, highest first:
#
#   1. what the user wrote (priority 100, the module system's default);
#   2. this module's plain default: nothing (an empty settings renders an
#      empty file, which the binary reads as its built-ins).
#
# The look is not a rung here at evaluation time at all: it arrives as a
# runtime layer under the user's own file (see `scoot look sync`), merged
# per key below every `settings` value above. So the rendered file holds
# only the user's `settings`, verbatim. The exception is the profile's
# `theme.overrides.effects`, which fills the file's `[effects]` table at
# `mkDefault`: those keys win per key over the look at runtime.
#
# Everything else the launcher reads has its own default in the binary,
# so an absent key is that default.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootlaunch;
  tomlFormat = pkgs.formats.toml { };

  # The desktop profile (`programs.scoot.desktop` in the scoot modules),
  # when those are imported beside this one. Each `or` keeps this
  # evaluating with the scoot module absent (as in this file's own
  # checks): nothing here requires it. The profile never sets across the
  # boundary in the other direction either (a conditional set of an
  # undeclared option fails eval whatever the condition is), so this read
  # is the whole launcher half of the profile.
  desktopProfile = ((config.programs.scoot or { }).desktop or { });
  profileOn = desktopProfile.enable or false;
  launcherOn = (desktopProfile.launcher or { }).enable or false;
  daemonIsScootLaunch = ((desktopProfile.launcher or { }).daemon or "fuzzel") == "scootlaunch";
  # The profile's look-independent depth and motion tokens
  # (`theme.overrides.effects`), written into the file's `[effects]`
  # table, which wins per key over the look. The notifier's card and title
  # tables are not the launcher's: the launcher refuses a file that
  # carries them.
  effectsOverride = ((desktopProfile.theme or { }).overrides or { }).effects or { };
in
{
  options.programs.scootlaunch = {
    enable = lib.mkEnableOption "scootlaunch, the keyboard-first application launcher";

    # `pkgs.scootlaunch` with the flake's overlay, else null; the flake's
    # wrappers (`homeModules.scootlaunch`, `nixosModules.scootlaunch`)
    # fill it with the flake's own build. Null with `enable` is refused:
    # the binds would have nothing to run.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scootlaunch or null;
      defaultText = lib.literalExpression "pkgs.scootlaunch or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scootlaunch";
      description = ''
        The scootlaunch package. Its Cargo features are `features`, below;
        a `package` you build yourself is used as given when `features`
        is null.
      '';
    };

    # The modules are Cargo features (`svg` on by default;
    # site/src/content/docs/scootlaunch/index.md). Null builds what
    # `package` is; a list is exactly the features that build has, so
    # `[ ]` is a launcher that draws a letter tile for SVG-only icons.
    features = lib.mkOption {
      type = lib.types.nullOr (lib.types.listOf lib.types.str);
      default = null;
      example = [ "svg" ];
      description = ''
        The features to build the launcher with, as Cargo features:
        exactly the ones listed (`scootlaunch.override {
        buildNoDefaultFeatures = true; buildFeatures = features; }`), so
        an empty list is a build without SVG icons. Null keeps `package`
        as it is (its default features). Needs a `package` that takes
        those overrides, as the flake's does.
      '';
    };

    # Free-form: a new option never needs a module change first, and the
    # file's schema is the launcher's own (`crates/scootlaunch` refuses
    # an unknown key by name, and the check in this flake runs the real
    # binary over the rendered file).
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        width_em = 50;
        rows = 12;
      };
      description = ''
        The contents of `launch.toml`, rendered as TOML. The options are
        the launcher's own (`font`, `width_em`, `rows`): what is not set
        takes the binary's own default, unless a look layer carries it at
        runtime (the active look through `scoot look sync`).
      '';
    };

    # Read-only results the two sides use.
    finalPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      readOnly = true;
      internal = true;
      description = "`package` with `features` applied.";
    };
    configFile = lib.mkOption {
      type = lib.types.package;
      readOnly = true;
      internal = true;
      description = "The rendered `launch.toml`.";
    };
  };

  config = lib.mkMerge [
    {
      programs.scootlaunch = {
        finalPackage =
          if cfg.package == null || cfg.features == null then
            cfg.package
          else
            cfg.package.override {
              buildNoDefaultFeatures = true;
              buildFeatures = cfg.features;
            };
        configFile = tomlFormat.generate "scootlaunch-launch.toml" cfg.settings;
      };
    }

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.package != null;
          message = ''
            programs.scootlaunch.package is null: apply the flake's overlay
            (inputs.scoot.overlays.default), import the flake's module
            (inputs.scoot.homeModules.scootlaunch / nixosModules.scootlaunch) or set
            the option, so the launcher has a binary to run.
          '';
        }
        {
          assertion = cfg.features == null || cfg.package == null || cfg.package ? override;
          message = ''
            programs.scootlaunch.features needs a package with `.override`
            (the flake's `scootlaunch`, which takes buildFeatures); the given
            package has none. Leave features null, or build the package
            with the features you want.
          '';
        }
      ];
    })

    # `theme.overrides.effects`: per key at `mkDefault`, so an explicit
    # `settings.effects.<key>` wins. Unset writes nothing.
    (lib.mkIf (effectsOverride != { }) {
      programs.scootlaunch.settings.effects = lib.mapAttrs (_: lib.mkDefault) effectsOverride;
    })

    # The desktop profile's launcher half: with the profile on, the
    # launcher slot on, and `desktop.launcher.daemon = "scootlaunch"`,
    # this launcher turns on too (at `mkDefault`, so an explicit value
    # still wins). The look reaches it as a runtime layer under the
    # user's own file (see `scoot look sync`), never as eval-time values
    # here.
    (lib.mkIf (profileOn && launcherOn && daemonIsScootLaunch) {
      programs.scootlaunch.enable = lib.mkDefault true;
    })
  ];
}
