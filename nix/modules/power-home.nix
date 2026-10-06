# `programs.scoot.desktop.power` for home-manager: the
# `powerprofilesctl` CLI behind the keymap's profile bind, and the
# charge-mode button's fill unit. The option shapes live in
# `./desktop.nix` (shared with the NixOS side); the `package` default
# lives here because only this side has `pkgs`. The daemons
# themselves (PPD, logind, UPower, the charge service) are the NixOS
# side's (`nixos.nix`): without it the CLI and the bind sit ready for
# a hand-written setup, the way a `[wallpaper]` finds scootbg on PATH
# without the home-manager side. See
# site/src/content/docs/desktop/index.md#power ("Power").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  power = cfg.desktop.power;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # The bar's build, when its module is imported beside this one (the
  # `or {}` keeps this evaluating without it, the way `scootbar.nix`
  # reads the profile). The fill unit below only exists then: without
  # a bar there is no button to fill.
  barBin = (config.programs.scootbar or { }).finalPackage or null;
in
{
  options.programs.scoot.desktop.power.profiles = {
    # The tool below is Linux-only: its attribute refuses evaluation
    # when forced on Darwin, so `or null` alone does not save it (the
    # Darwin `nix flake check` run reads every default). Off Linux it
    # defaults to null, which the assertion below refuses loudly
    # instead of installing nothing silently.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default =
        if isLinux then
          (if pkgs ? power-profiles-daemon then pkgs.power-profiles-daemon else null)
        else
          null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.power-profiles-daemon else null";
      description = ''
        The power-profiles-daemon package behind the keymap's profile
        bind (`powerprofilesctl`, which `scoot-power-profile` wraps).
        Null installs nothing. Linux-only: null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The CLI on PATH (the bind calls it through the wrapper script
    # beside the keymap, which names this package by absolute store
    # path).
    (lib.mkIf power.enable {
      assertions = [
        {
          assertion = power.profiles.package != null;
          message = ''
            programs.scoot.desktop.power.enable is set but
            programs.scoot.desktop.power.profiles.package is null: set
            it explicitly (apply the overlay, or point at a
            power-profiles-daemon).
          '';
        }
      ];

      home.packages = lib.optional (power.profiles.package != null) power.profiles.package;
    })

    # The charge cap rides with the policy (still individually
    # disable-able at plain priority, the way the lock rides with the
    # idle policy). The service itself is the NixOS side's; here this
    # only arms the bar button's fill unit below (which calls the
    # system-wide `scoot-charge` -- a home-manager-only setup provides
    # it on PATH some other way).
    (lib.mkIf power.enable {
      programs.scoot.desktop.power.chargeLimit.enable = lib.mkDefault true;
    })

    # The charge and low-battery bounds, refused on this side as well
    # as the NixOS one (same messages): a home-manager-only setup
    # must hear about a bad limit without the system module. Kept
    # outside `power.enable` so they still fire then, the way the
    # idle policy's refusals sit outside its own switch.
    (lib.mkIf power.chargeLimit.enable {
      assertions = [
        {
          assertion = power.chargeLimit.limit >= 1 && power.chargeLimit.limit <= 100;
          message = ''
            programs.scoot.desktop.power.chargeLimit.limit is a charge
            percent, 1 to 100.
          '';
        }
        {
          assertion =
            power.chargeLimit.battery == null
            || builtins.match "^[[:space:]]*$" power.chargeLimit.battery == null;
          message = ''
            programs.scoot.desktop.power.chargeLimit.battery is empty
            or blank: set the kernel's battery name (e.g. `BAT0`), or
            leave it null to auto-detect.
          '';
        }
      ];
    })
    (lib.mkIf power.enable {
      assertions = [
        {
          assertion = power.lowBattery.percentage >= 0 && power.lowBattery.percentage <= 5;
          message = ''
            programs.scoot.desktop.power.lowBattery.percentage is a
            battery percent at or under 5: UPower keeps `PercentageLow`
            20 and `PercentageCritical` 5, and anything above 5 breaks
            the descending order, so UPower would silently use its own
            triple instead.
          '';
        }
      ];
    })

    # Fill the charge-mode button when the bar starts (a push module
    # shows nothing until set): the charge service pushes on every
    # sync, but a fresh login would wait up to five minutes for one.
    # Beside the bar's own unit (`PartOf`, so it never lingers without
    # the bar), calling the system-wide `scoot-charge` by bare name --
    # the NixOS side installs it; a home-manager-only setup needs it
    # on PATH some other way (its absence fails this oneshot loudly
    # in `systemctl --user status`, nothing else).
    (lib.mkIf (power.enable && power.chargeLimit.enable && barBin != null) {
      systemd.user.services.scoot-charge-push = {
        Unit = {
          Description = "Show the battery charge mode in the bar";
          After = [ "scootbar.service" ];
          PartOf = [ "scootbar.service" ];
        };
        Service = {
          Type = "oneshot";
          ExecStartPre = "${lib.getExe' pkgs.coreutils "sleep"} 2";
          ExecStart = "scoot-charge push";
        };
        Install.WantedBy = [ "scootbar.service" ];
      };
    })
  ];
}
