# `programs.scoot-globalshortcuts-portal` for NixOS: the package, the
# D-Bus activation package, the `.portal` file package and a systemd
# user service for every user's graphical session. The options are in
# ./scoot-globalshortcuts-portal.nix; the home-manager side is
# ./scoot-globalshortcuts-portal-home.nix.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot-globalshortcuts-portal;

  # Which session scope starts the unit: the profile-managed backend in
  # scoot's own session only, a standalone backend in any graphical
  # session (same rule as the notification daemon's NixOS side).
  desktopProfile = ((config.programs.scoot or { }).desktop or { });
  sessionTarget =
    if (desktopProfile.enable or false) then "scoot-session.target" else "graphical-session.target";

  # The D-Bus activation file, rewritten to this build's binary (the
  # tree's own copy names `/usr/bin`). A package carrying it, so
  # `services.dbus.packages` publishes it on the session bus: with it,
  # a portal call with the backend down starts the unit instead of
  # failing. Installed here only while the backend is enabled.
  dbusService = pkgs.writeText "org.freedesktop.impl.portal.desktop.scoot.service" ''
    [D-BUS Service]
    Name=org.freedesktop.impl.portal.desktop.scoot
    Exec=${lib.getExe cfg.finalPackage} daemon
    SystemdService=scoot-globalshortcuts-portal.service
  '';
  dbusPackage = pkgs.runCommand "scoot-globalshortcuts-portal-dbus" { } ''
    mkdir -p $out/share/dbus-1/services
    cp ${dbusService} $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service
    mkdir -p $out/share/xdg-desktop-portal/portals
    cp ${portalFile} $out/share/xdg-desktop-portal/portals/scoot.portal
  '';

  # The portal file xdg-desktop-portal reads: which backend serves
  # GlobalShortcuts on scoot.
  portalFile = pkgs.writeText "scoot.portal" ''
    [portal]
    DBusName=org.freedesktop.impl.portal.desktop.scoot
    Interfaces=org.freedesktop.impl.portal.GlobalShortcuts;
    UseIn=scoot
  '';
in
{
  imports = [ ./scoot-globalshortcuts-portal.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    services.dbus.packages = lib.optional (cfg.finalPackage != null) dbusPackage;

    systemd.user.services.scoot-globalshortcuts-portal =
      lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null)
        {
          description = "scoot GlobalShortcuts portal backend";
          wantedBy = [ sessionTarget ];
          partOf = [ sessionTarget ];
          after = [ sessionTarget ];
          unitConfig.StartLimitIntervalSec = 0;
          serviceConfig = {
            Type = "dbus";
            BusName = "org.freedesktop.impl.portal.desktop.scoot";
            ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
            ExecStart = "${lib.getExe cfg.finalPackage} daemon";
            Restart = "on-failure";
            RestartSec = 1;
            # A taken name is the backend's exit 3: a stopped unit, never
            # a restart loop.
            RestartPreventExitStatus = 3;
          };
        };
  };
}
