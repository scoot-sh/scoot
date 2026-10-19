# `programs.scootlaunch` for home-manager: the config file. The options
# and the rendered file are in ./scootlaunch.nix; the NixOS side is
# ./scootlaunch-nixos.nix. See site/src/content/docs/scootlaunch/index.md.
{ config, lib, ... }:

let
  cfg = config.programs.scootlaunch;
in
{
  imports = [ ./scootlaunch.nix ];

  config = lib.mkIf cfg.enable {
    home.packages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # The launcher's default path, so `scootlaunch` from a shell reads
    # the same file the binds open, and `--check` validates it.
    xdg.configFile."scoot/launch.toml".source = cfg.configFile;
  };
}
