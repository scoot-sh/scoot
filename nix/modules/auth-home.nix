# `programs.scoot.desktop.auth` and `.secrets` for home-manager: the
# polkit agent's package and the secrets client's tools. The option
# shapes live in `./desktop.nix` (shared with the NixOS side); the
# `package` defaults live here because only this side has `pkgs`.
#
# Deliberately NO user unit for the agent: a polkit agent registers
# against its own logind session, and a systemd user unit never joins
# one (proven live: the unit's agent stays connected but unregistered,
# and registration fails with "User of caller and user of subject
# differs"). The agent runs instead as a child of the session leader
# (`resources/scoot-session`, in the session scope -- see
# `SCOOT_POLKIT_AGENT` there), named by the NixOS side's session
# entry. This side installs its package (for the entry's absolute
# path, and for hand runs) beside the keyring client below.
#
# The polkit authority (polkitd) and the keyring's D-Bus activation
# files are the NixOS side's (`nixos.nix`): without it the agent has
# nobody to answer and secrets have nothing to activate. See
# site/src/content/docs/desktop/index.md#privilege-prompts-and-the-keyring.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  auth = cfg.desktop.auth;
  secrets = cfg.desktop.secrets;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # One agent package per `daemon`, so the default follows the pick.
  # Each guarded off Linux the way every other slot's tool is (the
  # attributes refuse evaluation when forced on Darwin, so the Darwin
  # `nix flake check` run reads every default): off Linux each is
  # null, which the assertions below refuse loudly.
  agentPackages = {
    gnome = if isLinux then (if pkgs ? polkit_gnome then pkgs.polkit_gnome else null) else null;
    lxqt =
      if isLinux then
        (if (pkgs.lxqt or { }) ? lxqt-policykit then pkgs.lxqt.lxqt-policykit else null)
      else
        null;
    hyprpolkit =
      if isLinux then (if pkgs ? hyprpolkitagent then pkgs.hyprpolkitagent else null) else null;
  };
in
{
  options.programs.scoot.desktop.auth = {
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = agentPackages.${auth.daemon} or null;
      defaultText = lib.literalExpression "per-daemon (polkit_gnome, lxqt-policykit or hyprpolkitagent)";
      description = ''
        The polkit agent package the session entry names for
        `auth.daemon`. Follows the daemon pick; point at your own
        build to override. Null installs nothing. Linux-only: null
        off Linux.
      '';
    };
  };

  options.programs.scoot.desktop.secrets = {
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? gnome-keyring then pkgs.gnome-keyring else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.gnome-keyring else null";
      description = ''
        The keyring package behind `daemon` (its daemon, its PAM
        module, its D-Bus activation files). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    # `secret-tool` lives in libsecret, not in gnome-keyring itself:
    # without this on PATH the ticket's own acceptance (`secret-tool
    # store`) has nothing to call. Small C CLI over the same
    # `org.freedesktop.secrets` name the daemon owns.
    clientPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? libsecret then pkgs.libsecret else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.libsecret else null";
      description = ''
        The secrets client to install (`secret-tool`, from
        libsecret). Null installs nothing. Linux-only: null off
        Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The agent: its package on PATH (the session entry calls it by
    # absolute store path; installing it here keeps hand runs and
    # version checks working too).
    (lib.mkIf auth.enable {
      assertions = [
        {
          assertion = auth.package != null;
          message = ''
            programs.scoot.desktop.auth.enable is set but
            programs.scoot.desktop.auth.package is null: set it
            explicitly (apply the overlay, or point at a polkit
            agent).
          '';
        }
      ];

      home.packages = lib.optional (auth.package != null) auth.package;
    })

    # The secrets client beside the slot (the daemon itself is D-Bus
    # activated from the NixOS side's files -- no unit here, by
    # design: it starts on the first secrets call and holds the
    # unlocked keyring until the session ends).
    (lib.mkIf secrets.enable {
      assertions = [
        {
          assertion = secrets.package != null;
          message = ''
            programs.scoot.desktop.secrets.enable is set but
            programs.scoot.desktop.secrets.package is null: set it
            explicitly (apply the overlay, or point at a
            gnome-keyring).
          '';
        }
        {
          assertion = secrets.clientPackage != null;
          message = ''
            programs.scoot.desktop.secrets.enable is set but
            programs.scoot.desktop.secrets.clientPackage is null:
            set it explicitly (apply the overlay, or point at a
            libsecret).
          '';
        }
      ];

      home.packages = lib.optional (secrets.clientPackage != null) secrets.clientPackage;
    })

    # The profile turns both slots on (each still individually
    # disable-able at plain priority, the way the notification daemon
    # works).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.auth.enable = lib.mkDefault true;
      programs.scoot.desktop.secrets.enable = lib.mkDefault true;
    })
  ];
}
