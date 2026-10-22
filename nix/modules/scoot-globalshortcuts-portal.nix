# The part of `programs.scoot-globalshortcuts-portal` that home-manager
# and NixOS share: the options and the package with its Cargo features.
# `scoot-globalshortcuts-portal-home.nix` and
# `scoot-globalshortcuts-portal-nixos.nix` import it and add only what
# their side has (the unit, the D-Bus activation file, the `.portal`
# file). The backend reads no config file of its own: assignment lives
# in the compositor's `[global_shortcuts]` (see
# dev/design/global-shortcuts.md), so there is no `settings` here.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot-globalshortcuts-portal;
in
{
  options.programs.scoot-globalshortcuts-portal = {
    enable = lib.mkEnableOption "scoot-globalshortcuts-portal, the GlobalShortcuts portal backend";

    # `pkgs.scoot-globalshortcuts-portal` with the flake's overlay, else
    # null; the flake's wrappers (`homeModules` /
    # `nixosModules.scoot-globalshortcuts-portal`) fill it with the
    # flake's own build. Null with `enable` is refused: the unit would
    # have nothing to run.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot-globalshortcuts-portal or null;
      defaultText = lib.literalExpression "pkgs.scoot-globalshortcuts-portal or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scoot-globalshortcuts-portal";
      description = ''
        The portal backend package. Its Cargo features are `features`, below;
        a `package` you build yourself is used as given when `features`
        is null.
      '';
    };

    # The modules are Cargo features (none today: the backend is one
    # build -- see nix/scoot-globalshortcuts-portal.nix). Null builds
    # what `package` is; a list is exactly the features that build has.
    features = lib.mkOption {
      type = lib.types.nullOr (lib.types.listOf lib.types.str);
      default = null;
      example = [ ];
      description = ''
        The features to build the backend with, as Cargo features:
        exactly the ones listed. Null keeps `package` as it is (its
        default features). Needs a `package` that takes those overrides,
        as the flake's does.
      '';
    };

    # Run the backend as a systemd user service. On to run it
    # (`scoot-globalshortcuts-portal.service`, wanted by the session
    # scope and restarted when it fails). Off installs the package and
    # the files, and starts the backend some other way (D-Bus activation
    # starts it on the first portal call either way).
    systemd.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run the backend as a systemd user service,
        `scoot-globalshortcuts-portal.service`, wanted by the session
        scope (`scoot-session.target` under the desktop profile,
        `graphical-session.target` standalone). Turn it off to start
        `scoot-globalshortcuts-portal daemon` some other way.
      '';
    };

    # Read-only result the two sides use.
    finalPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      readOnly = true;
      internal = true;
      description = "`package` with `features` applied.";
    };
  };

  config = lib.mkMerge [
    {
      programs.scoot-globalshortcuts-portal = {
        finalPackage =
          if cfg.package == null || cfg.features == null then
            cfg.package
          else
            cfg.package.override {
              buildNoDefaultFeatures = true;
              buildFeatures = cfg.features;
            };
      };
    }

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.package != null;
          message = ''
            programs.scoot-globalshortcuts-portal.package is null: apply the flake's overlay
            (inputs.scoot.overlays.default), import the flake's module
            (inputs.scoot.homeModules.scoot-globalshortcuts-portal / nixosModules.scoot-globalshortcuts-portal) or set
            the option, so the service has a backend to run.
          '';
        }
        {
          assertion = cfg.features == null || cfg.package == null || cfg.package ? override;
          message = ''
            programs.scoot-globalshortcuts-portal.features needs a package with `.override`
            (the flake's `scoot-globalshortcuts-portal`, which takes buildFeatures); the given
            package has none. Leave features null, or build the package
            with the features you want.
          '';
        }
      ];
    })
  ];
}
