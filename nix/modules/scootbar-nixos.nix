# `programs.scootbar` for NixOS: the package, the config file in `/etc`
# and a systemd user service for every user's graphical session. The
# options and the Stylix defaults are in ./scootbar.nix; the home-manager
# side (per-user, the file at the bar's default path) is
# ./scootbar-home.nix. See docs/nix.md.
{ config, lib, ... }:

let
  cfg = config.programs.scootbar;
in
{
  imports = [ ./scootbar.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # Not `/etc/xdg`: the bar reads only `$XDG_CONFIG_HOME/scoot/bar.toml`,
    # so the unit names this file with `--config`. A `scootbar daemon`
    # started by hand reads the user's own file, not this one.
    environment.etc."scootbar/bar.toml".source = cfg.configFile;

    systemd.user.services.scootbar = lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null) {
      description = "scootbar, the status bar";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      # Before what hosts a tray, where the session has such a target.
      before = [ "tray.target" ];
      # Retry for as long as the compositor is not there. systemd's default
      # burst limit (5 starts in 10 s) is not reached by a 2 s retry (measured,
      # scripts/scootbar-unit-test.sh S7, systemd 261), so this is insurance: it keeps the
      # retries unending if RestartSec is ever lowered.
      unitConfig.StartLimitIntervalSec = 0;
      restartTriggers = [ cfg.configFile ];
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.finalPackage} daemon --config /etc/scootbar/bar.toml";
        Restart = "on-failure";
        RestartSec = 2;
      };
    };
  };
}
