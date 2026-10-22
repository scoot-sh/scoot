# `programs.scoot-globalshortcuts-portal` for home-manager: the user
# unit, the D-Bus activation file and the `.portal` file. The options
# are in ./scoot-globalshortcuts-portal.nix; the NixOS side is
# ./scoot-globalshortcuts-portal-nixos.nix.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot-globalshortcuts-portal;

  # The D-Bus activation file: with it on the data path, a portal call
  # with the backend down starts the unit instead of failing. Rewritten
  # to this build's binary (the tree's own copy names `/usr/bin`).
  # Never installed beside another GlobalShortcuts backend's: with two
  # files for the name on the bus's search path the bus activates one
  # of them, and which one is not something to rely on -- so this side
  # installs it only while the backend is enabled.
  dbusService = pkgs.writeText "org.freedesktop.impl.portal.desktop.scoot.service" ''
    [D-BUS Service]
    Name=org.freedesktop.impl.portal.desktop.scoot
    Exec=${lib.getExe cfg.finalPackage} daemon
    SystemdService=scoot-globalshortcuts-portal.service
  '';

  # The portal file: tells xdg-desktop-portal this backend serves
  # GlobalShortcuts on scoot (`UseIn=scoot`). Rewritten to this build's
  # binary? No -- the `.portal` file names only the bus name; activation
  # goes through the D-Bus service above.
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
    home.packages = lib.optional (cfg.finalPackage != null) cfg.finalPackage;

    # The activation file beside the unit: the data path the session
    # bus searches.
    xdg.dataFile."dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service".source = lib.mkIf (
      cfg.finalPackage != null
    ) dbusService;

    # The portal file xdg-desktop-portal reads: the per-user portals
    # lookup slot.
    xdg.dataFile."xdg-desktop-portal/portals/scoot.portal".source = lib.mkIf (
      cfg.finalPackage != null
    ) portalFile;

    systemd.user.services.scoot-globalshortcuts-portal =
      lib.mkIf (cfg.systemd.enable && cfg.finalPackage != null)
        {
          Unit = {
            Description = "scoot GlobalShortcuts portal backend";
            PartOf = [ "scoot-session.target" ];
            After = [ "scoot-session.target" ];
            # Unending retries, like the bar's unit: `StartLimitIntervalSec`
            # lives here in `[Unit]` (systemd ignores it in `[Service]`).
            StartLimitIntervalSec = 0;
          };
          Service = {
            # D-Bus activation (`Type=dbus` plus the name): the unit is
            # started when claimed, and considered started once the backend
            # owns the name.
            Type = "dbus";
            BusName = "org.freedesktop.impl.portal.desktop.scoot";
            # Activation can arrive before the session reaches the
            # session target (a portal call in an early autostart): skip
            # cleanly then -- no restart -- and the next call
            # re-activates. The wanted-by below starts it with the
            # display in the common case.
            ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
            ExecStart = "${lib.getExe cfg.finalPackage} daemon";
            Restart = "on-failure";
            RestartSec = 1;
            # A taken name (another GlobalShortcuts backend still running)
            # is the backend's exit 3: a stopped unit, never a restart
            # loop (a restart would spin against the owner until the burst
            # limit).
            RestartPreventExitStatus = 3;
          };
          Install.WantedBy = [ "scoot-session.target" ];
        };
  };
}
