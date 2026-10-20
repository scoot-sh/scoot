# `programs.scootnotify` for NixOS: the package, the config file in
# `/etc`, the D-Bus activation package and a systemd user service for
# every user's graphical session. The options and the rendered file are
# in ./scootnotify.nix; the home-manager side (per-user, the file at
# the daemon's default path) is ./scootnotify-home.nix.
# See site/src/content/docs/scootnotify/index.md.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootnotify;

  # Which session scope starts the unit: the profile-managed daemon in
  # scoot's own session only, a standalone daemon in any graphical
  # session (same rule as the bar's NixOS side -- see
  # `scootbar-nixos.nix`).
  desktopProfile = ((config.programs.scoot or { }).desktop or { });
  sessionTarget =
    if (desktopProfile.enable or false) then "scoot-session.target" else "graphical-session.target";

  # The D-Bus activation file, rewritten to this build's binary (the
  # tree's own copy names `/usr/bin`). A package carrying it, so
  # `services.dbus.packages` publishes it on the session bus: with it,
  # a `Notify` with the daemon down starts the unit instead of failing.
  # Never installed beside mako's or dunst's (see the home-manager
  # side): installed here only while this daemon is enabled.
  dbusService = pkgs.writeText "org.freedesktop.Notifications.service" ''
    [D-BUS Service]
    Name=org.freedesktop.Notifications
    Exec=${lib.getExe cfg.finalPackage} daemon
    SystemdService=scootnotify.service
  '';
  dbusPackage = pkgs.runCommand "scootnotify-dbus" { } ''
    mkdir -p $out/share/dbus-1/services
    cp ${dbusService} $out/share/dbus-1/services/org.freedesktop.Notifications.service
  '';
in
{
  imports = [ ./scootnotify.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # Not `/etc/xdg`: the daemon reads only `$XDG_CONFIG_HOME/scoot/notify.toml`,
    # so the unit names this file with `--config`. A `scootnotify daemon`
    # started by hand reads the user's own file, not this one.
    environment.etc."scoot/notify.toml".source = lib.mkIf (cfg.finalPackage != null) cfg.configFile;

    services.dbus.packages = lib.optional (cfg.finalPackage != null) dbusPackage;

    systemd.user.services.scootnotify = lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null) {
      description = "scootnotify notification daemon (desktop.notifications)";
      wantedBy = [ sessionTarget ];
      partOf = [ sessionTarget ];
      after = [ sessionTarget ];
      # A new config reloads the daemon in place (never a restart: a
      # restart drops the cards and the history). `reloadIfChanged`
      # stays off: with it on, `reloadTriggers` would behave like
      # `restartTriggers` (nixpkgs warns when both are set).
      reloadTriggers = [ cfg.configFile ];
      unitConfig.StartLimitIntervalSec = 0;
      serviceConfig = {
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
        ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
        ExecStart = "${lib.getExe cfg.finalPackage} daemon --config /etc/scoot/notify.toml";
        ExecReload = "${lib.getExe cfg.finalPackage} msg reload";
        Restart = "on-failure";
        RestartSec = 1;
        # A taken name (mako, dunst, swaync still running) is the
        # daemon's exit 3: a stopped unit, never a restart loop.
        RestartPreventExitStatus = 3;
      };
    };
  };
}
