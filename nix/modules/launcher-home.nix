# `programs.scoot.desktop.launcher` for home-manager: the fuzzel
# package behind the keymap's launcher binds. The option shapes live in
# `./desktop.nix` (shared with the NixOS side); the `package` default
# lives here because only this side has `pkgs`. See docs/nix.md
# ("Launcher").
#
# The launcher itself is a wrapper script beside the binds
# (`keys-home.nix` owns it the way it owns the clipboard picker and the
# capture scripts: the script needs the keymap's bind context): this
# side installs the package it runs, which is the one fuzzel the
# clipboard picker already themes (same derivation, same flags -- see
# `fuzzel-theme.nix`), so the slot adds no second copy.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  launch = cfg.desktop.launcher;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;
in
{
  options.programs.scoot.desktop.launcher = {
    # The tool below is Linux-only: its attribute refuses evaluation
    # when forced on Darwin, so `or null` alone does not save it (the
    # Darwin `nix flake check` run reads every default). Off Linux it
    # defaults to null, which the assertion below refuses loudly
    # instead of installing nothing silently.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? fuzzel then pkgs.fuzzel else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel else null";
      description = ''
        The launcher package to run the keymap's binds from (must
        speak fuzzel's flags and `--dmenu`: the bar's future pickers
        call that contract, which a future scootlaunch keeps). Null
        installs nothing. Linux-only: null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The launcher: its package on PATH (the binds call it through the
    # wrapper script beside the keymap, which names this package by
    # absolute store path). No daemon, no unit, no config file: the
    # launcher holds nothing when closed.
    (lib.mkIf launch.enable {
      assertions = [
        {
          assertion = launch.package != null;
          message = ''
            programs.scoot.desktop.launcher.enable is set but
            programs.scoot.desktop.launcher.package is null: set it
            explicitly (apply the overlay, or point at a fuzzel).
          '';
        }
      ];

      home.packages = lib.optional (launch.package != null) launch.package;
    })

    # The profile turns the slot on (still individually disable-able at
    # plain priority, the way the notification daemon works).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.launcher.enable = lib.mkDefault true;
    })
  ];
}
