# `programs.scootbar` for home-manager: the config file and the systemd
# user service. The options and the Stylix defaults are in ./scootbar.nix;
# the NixOS side is ./scootbar-nixos.nix. See docs/nix.md.
{ config, lib, ... }:

let
  cfg = config.programs.scootbar;
in
{
  imports = [ ./scootbar.nix ];

  config = lib.mkIf cfg.enable {
    home.packages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # The bar's default path, so `scootbar daemon` from a shell or an
    # autostart reads the same file the unit does, and `scootbar msg
    # reload` re-reads it.
    xdg.configFile."scoot/bar.toml".source = cfg.configFile;

    systemd.user.services.scootbar = lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null) {
      Unit = {
        Description = "scootbar, the status bar";
        # Up with the graphical session (which a scoot session script
        # starts after importing WAYLAND_DISPLAY: docs/nix.md), and before what hosts a tray (`tray.target`, where the session
        # defines one; an ordering against an absent unit does nothing).
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        Before = [ "tray.target" ];
        # Retry for as long as the compositor is not there. systemd's default
        # burst limit (5 starts in 10 s) is not reached by a 2 s retry
        # (measured, scripts/scootbar-unit-test.sh S7, systemd 261), so this is insurance:
        # it keeps the retries unending if RestartSec is ever lowered.
        StartLimitIntervalSec = 0;
        # A new config restarts the bar (it starts in milliseconds).
        X-Restart-Triggers = [ "${cfg.configFile}" ];
      };
      Service = {
        ExecStart = "${lib.getExe cfg.finalPackage} daemon";
        # `on-failure`: a crash, or the compositor going away, brings it
        # back, and so does a start before WAYLAND_DISPLAY is imported (no
        # `ConditionEnvironment`: a skipped start is never retried);
        # `scootbar msg kill` and a stop stay stopped. Two seconds so a
        # compositor that is not up yet is not spun against.
        Restart = "on-failure";
        RestartSec = 2;
        # A bar button or a binding launches apps as the bar's children
        # (docs/scootbar/cli.md#pointer-input). With the default
        # `control-group` every restart of the bar (a config change, a
        # crash) would kill them with it; `process` stops only the bar.
        KillMode = "process";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
