# Checks for the portal backend's NixOS/home-manager modules, wired
# into `nix flake check` as `checks.<system>.portal-modules` (Linux
# only: the backend does not build elsewhere).
#
# What this pins:
# - `enable = false` manages nothing; a null package is refused by name;
# - `features` reaches the build as `buildNoDefaultFeatures` +
#   `buildFeatures`, and an empty list builds the one backend as is;
# - the user unit: `Type=dbus` with the bus name, `ExecStart` the
#   daemon, `RestartPreventExitStatus=3` (a taken name is a stopped
#   unit, never a restart loop), the WAYLAND_DISPLAY condition, and the
#   session-scope binding; the D-Bus activation file names the same
#   binary with `SystemdService=scoot-globalshortcuts-portal.service`;
# - the `.portal` file names the backend bus name, the GlobalShortcuts
#   interface and `UseIn=scoot`.
{
  lib,
  pkgs,
  runCommand,
  python3,
  portal,
  homeModule,
  nixosModule,
}:

let
  baseStubs = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            assertion = lib.mkOption { type = lib.types.bool; };
            message = lib.mkOption { type = lib.types.str; };
          };
        }
      );
      default = [ ];
    };
    options.warnings = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
  };

  homeStubs = {
    options.home.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.xdg.dataFile = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.source = lib.mkOption { type = lib.types.path; };
        }
      );
      default = { };
    };
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };
  nixosStubs = {
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.services.dbus.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.programs.scoot.desktop.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
  };

  evalWith =
    { side }:
    cfg:
    lib.evalModules {
      modules = [
        (if side == "home" then homeModule else nixosModule)
        baseStubs
        (if side == "home" then homeStubs else nixosStubs)
        {
          programs.scoot-globalshortcuts-portal = cfg // {
            package = cfg.package or portal;
          };
        }
      ];
      specialArgs = { inherit pkgs; };
    };
  evalHome = evalWith { side = "home"; };
  evalNixos = evalWith { side = "nixos"; };

  failing = e: map (a: a.message) (lib.filter (a: !a.assertion) e.config.assertions);

  plain = evalHome { enable = true; };
  recorded = evalHome {
    enable = true;
    package = recorder;
    features = [ ];
  };
  unrecordable = evalHome {
    enable = true;
    package = fakePortal;
    features = [ ];
  };
  off = evalHome { };
  offUnit = evalHome {
    enable = true;
    systemd.enable = false;
  };
  nullPkg = evalHome {
    enable = true;
    package = null;
  };
  os = evalNixos { enable = true; };
  osOff = evalNixos { };
  osNoUnit = evalNixos {
    enable = true;
    systemd.enable = false;
  };

  # A stand-in package that records the arguments `.override` was given.
  fakePortal = pkgs.runCommand "fake-portal" { } ''
    mkdir -p $out/bin
    printf '#!/bin/sh\n' > $out/bin/scoot-globalshortcuts-portal
    chmod +x $out/bin/scoot-globalshortcuts-portal
  '';
  recorder = fakePortal // {
    override = args: fakePortal // { overrideArgs = args; };
  };

  homeSvc = plain.config.systemd.user.services.scoot-globalshortcuts-portal;
  osSvc = os.config.systemd.user.services.scoot-globalshortcuts-portal;
  activationSource =
    plain.config.xdg.dataFile."dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service".source;
  portalSource = plain.config.xdg.dataFile."xdg-desktop-portal/portals/scoot.portal".source;

  is = a: b: a == b;
  checks = {
    "features reaches the build as no-default-features + the list" =
      recorded.config.programs.scoot-globalshortcuts-portal.finalPackage.overrideArgs == {
        buildNoDefaultFeatures = true;
        buildFeatures = [ ];
      };
    "features null leaves the package untouched" =
      plain.config.programs.scoot-globalshortcuts-portal.finalPackage == portal;
    "features with a package that has no .override is refused by name" =
      lib.any (lib.hasInfix "programs.scoot-globalshortcuts-portal.features needs") (
        failing unrecordable
      );
    "enable = false manages nothing" =
      off.config.home.packages == [ ]
      && off.config.xdg.dataFile == { }
      && osOff.config.environment.systemPackages == [ ]
      && osOff.config.services.dbus.packages == [ ];
    "a null package with enable is refused by name" =
      lib.any (lib.hasInfix "programs.scoot-globalshortcuts-portal.package is null") (failing nullPkg);
    "home: package, activation file and portal file installed" =
      plain.config.home.packages == [ portal ]
      && plain.config.xdg.dataFile ? "dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service"
      && plain.config.xdg.dataFile ? "xdg-desktop-portal/portals/scoot.portal";
    "home: the unit is Type=dbus owning the backend name" =
      homeSvc.Service.Type == "dbus"
      && homeSvc.Service.BusName == "org.freedesktop.impl.portal.desktop.scoot";
    "home: ExecStart runs the daemon" =
      lib.hasInfix "/bin/scoot-globalshortcuts-portal daemon" homeSvc.Service.ExecStart;
    "home: a taken name stops the unit instead of restart-looping" =
      homeSvc.Service.RestartPreventExitStatus == 3;
    "home: the display condition and the session-scope binding" =
      lib.hasInfix "WAYLAND_DISPLAY" homeSvc.Service.ExecCondition
      && homeSvc.Unit.PartOf == [ "scoot-session.target" ]
      && homeSvc.Unit.After == [ "scoot-session.target" ]
      && homeSvc.Install.WantedBy == [ "scoot-session.target" ];
    "home: unending retries, with the burst key in [Unit] (never [Service])" =
      homeSvc.Unit.StartLimitIntervalSec == 0
      && !(homeSvc.Service ? StartLimitIntervalSec)
      && !(homeSvc.Service ? StartLimitBurst)
      && !(homeSvc.Unit ? StartLimitBurst);
    "home: systemd.enable = false keeps the package and the files, not the unit" =
      !(offUnit.config.systemd.user.services ? scoot-globalshortcuts-portal)
      && offUnit.config.xdg.dataFile
        ? "dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service";
    "nixos: package installed and the activation package published" =
      os.config.environment.systemPackages == [ portal ] && os.config.services.dbus.packages != [ ];
    "nixos: the unit owns the backend name and stops on a taken one" =
      osSvc.serviceConfig.Type == "dbus"
      && osSvc.serviceConfig.BusName == "org.freedesktop.impl.portal.desktop.scoot"
      && osSvc.serviceConfig.RestartPreventExitStatus == 3;
    "nixos: systemd.enable = false keeps the bus files, not the unit" =
      !(osNoUnit.config.systemd.user.services ? scoot-globalshortcuts-portal)
      && osNoUnit.config.services.dbus.packages != [ ];
  };
  failed = lib.attrNames (lib.filterAttrs (_: ok: !ok) checks);
in
assert lib.assertMsg (failed == [ ]) "portal module checks failed: ${builtins.toJSON failed}";
runCommand "portal-modules"
  {
    nativeBuildInputs = [ python3 ];
  }
  ''
    set -eu
    echo "--- activation file ---"
    cat "${activationSource}"
    grep -qx 'Name=org.freedesktop.impl.portal.desktop.scoot' "${activationSource}"
    grep -q 'scoot-globalshortcuts-portal.*daemon' "${activationSource}"
    grep -qx 'SystemdService=scoot-globalshortcuts-portal.service' "${activationSource}"
    echo "--- portal file ---"
    cat "${portalSource}"
    grep -qx 'DBusName=org.freedesktop.impl.portal.desktop.scoot' "${portalSource}"
    grep -qx 'Interfaces=org.freedesktop.impl.portal.GlobalShortcuts;' "${portalSource}"
    grep -qx 'UseIn=scoot' "${portalSource}"
    echo "ok: portal module files"
    touch $out
  ''
