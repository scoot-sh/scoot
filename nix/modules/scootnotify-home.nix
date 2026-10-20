# `programs.scootnotify` for home-manager: the config file, the user unit
# and the D-Bus activation file. The options and the rendered file are
# in ./scootnotify.nix; the NixOS side is ./scootnotify-nixos.nix.
# See site/src/content/docs/scootnotify/index.md.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootnotify;

  # The D-Bus activation file: with it on the data path, a `Notify`
  # with the daemon down starts the unit instead of failing. Rewritten
  # to this build's binary (the tree's own copy names `/usr/bin`).
  # Never installed beside mako's or dunst's: with two files for the
  # name on the bus's search path the bus activates one of them, and
  # which one is not something to rely on -- so this side installs it
  # only while this daemon is enabled, and the profile's mako half
  # stands down with it (see `notifications-home.nix`).
  dbusService = pkgs.writeText "org.freedesktop.Notifications.service" ''
    [D-BUS Service]
    Name=org.freedesktop.Notifications
    Exec=${lib.getExe cfg.finalPackage} daemon
    SystemdService=scootnotify.service
  '';
in
{
  imports = [ ./scootnotify.nix ];

  config = lib.mkIf cfg.enable {
    home.packages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # The daemon's default path, so `scootnotify daemon` from a shell
    # reads the same file the unit opens, and `daemon --check`
    # validates it.
    xdg.configFile."scoot/notify.toml".source = lib.mkIf (cfg.finalPackage != null) cfg.configFile;

    # The activation file beside the unit: the data path the session
    # bus searches.
    xdg.dataFile."dbus-1/services/org.freedesktop.Notifications.service".source = lib.mkIf (
      cfg.finalPackage != null
    ) dbusService;

    systemd.user.services.scootnotify = lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null) {
      Unit = {
        Description = "scootnotify notification daemon (desktop.notifications)";
        PartOf = [ "scoot-session.target" ];
        After = [ "scoot-session.target" ];
        # A new config reloads the daemon in place (never a restart:
        # a restart drops the cards and the history). The binary
        # answers `msg reload` with applied or refused, keeping the
        # last good config.
        X-Reload-Triggers = [ "${cfg.configFile}" ];
        # Unending retries, like the bar's unit: `StartLimitIntervalSec`
        # lives here in `[Unit]` (systemd ignores it in `[Service]`).
        StartLimitIntervalSec = 0;
      };
      Service = {
        # D-Bus activation (`Type=dbus` plus the name): the unit is
        # started when claimed, and considered started once the daemon
        # owns the name.
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
        # Activation can arrive before the session reaches the
        # session target (a `Notify` in an early autostart): skip
        # cleanly then -- no restart -- and the next `Notify`
        # re-activates. The wanted-by below starts it with the
        # display in the common case.
        ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
        ExecStart = "${lib.getExe cfg.finalPackage} daemon";
        ExecReload = "${lib.getExe cfg.finalPackage} msg reload";
        Restart = "on-failure";
        RestartSec = 1;
        # A taken name (mako, dunst, swaync still running) is the
        # daemon's exit 3: a stopped unit, never a restart loop (a
        # restart would spin against the owner until the burst limit).
        RestartPreventExitStatus = 3;
      };
      Install.WantedBy = [ "scoot-session.target" ];
    };
  };
}
