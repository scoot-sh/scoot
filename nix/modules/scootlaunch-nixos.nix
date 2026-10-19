# `programs.scootlaunch` for NixOS: the package and the config file in
# `/etc`. The options and the rendered file are in ./scootlaunch.nix;
# the home-manager side is ./scootlaunch-home.nix.
# See site/src/content/docs/scootlaunch/index.md.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootlaunch;
in
{
  imports = [ ./scootlaunch.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    environment.etc."scootlaunch/launch.toml".source = cfg.configFile;
  };
}
