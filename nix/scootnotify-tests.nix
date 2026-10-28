# Checks for the scootnotify NixOS/home-manager modules, wired into
# `nix flake check` as `checks.<system>.scootnotify-modules` (Linux only:
# the daemon does not build elsewhere). Same shape as
# ./scootlaunch-tests.nix: each module is evaluated standalone with stub
# options standing in for what home-manager and NixOS own, and the
# rendered files are validated by python's tomllib. Then the part no stub
# can stand in for: the REAL scootnotify binary reads each rendered file
# with `daemon --check` (what a start does before it owns the name, with
# no compositor to fake) and refuses a file with an unknown key -- the
# control that shows the check can fail.
#
# What this pins:
# - the rendered file holds only the user's `settings` (verbatim, no look
#   layer: the look arrives at runtime under the user's own file);
# - `enable = false` manages nothing; a null package is refused by name;
# - `features` reaches the build as `buildNoDefaultFeatures` +
#   `buildFeatures`, and an empty list builds the one daemon as is;
# - the user unit: `Type=dbus` with the bus name, `ExecStart` the daemon,
#   `ExecReload` a `msg reload` (never a restart: a new config reloads
#   in place, keeping the cards and the history),
#   `RestartPreventExitStatus=3` (a taken name is a stopped unit, never
#   a restart loop), the WAYLAND_DISPLAY condition, and the
#   `scoot-session.target` binding; the D-Bus activation file names the
#   same binary with `SystemdService=scootnotify.service`;
# - the desktop profile's default (`desktop.notifications.daemon =
#   "scootnotify"`, through a stub of that option subtree) enables this
#   daemon at `mkDefault` (an explicit value still wins), while
#   `daemon = "mako"` leaves it alone;
# - a non-empty mako-shaped `desktop.notifications.settings` beside the
#   new daemon warns, naming `programs.scootnotify.settings` (the one
#   slot option that does not survive the swap);
# - there is no Stylix switch (Stylix reaches it only as the `stylix`
#   look);
# - the desktop profile's `theme.targets.notifications.enable = false`
#   beside this daemon renders `look = false` (at `mkDefault`: an
#   explicit `settings.look` wins), on both sides; on (the default), with
#   the mako daemon, or with the profile off, the file carries no look
#   key -- and the real binary accepts the key (see the next item);
# - the real binary accepts every rendered file (`scootnotify daemon
#   --check`, exit 0) and refuses a file with an unknown key (exit 1).
{
  lib,
  pkgs,
  runCommand,
  python3,
  scootnotify,
  homeModule,
  nixosModule,
}:

let
  tomlFormat = pkgs.formats.toml { };

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
    options.lib = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };

  homeStubs = {
    options.home.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.xdg.configFile = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.source = lib.mkOption { type = lib.types.path; };
        }
      );
      default = { };
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
    options.environment.etc = lib.mkOption {
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
    options.services.dbus.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
  };

  # The desktop profile as the auto-enable reads it: only the leaves
  # this module asks of `programs.scoot` (see
  # nix/modules/scootnotify.nix). The same types and defaults as the
  # real option subtree in nix/modules/desktop.nix, so a default here
  # means the default there.
  profileStub =
    {
      enable ? false,
      notifications ? true,
      daemon ? "scootnotify",
      settings ? { },
      # `theme.targets.notifications.enable`: the real option's default.
      lookTarget ? true,
    }:
    {
      options.programs.scoot.desktop.theme.targets.notifications.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
      };
      options.programs.scoot.desktop.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.programs.scoot.desktop.notifications.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.programs.scoot.desktop.notifications.daemon = lib.mkOption {
        type = lib.types.enum [
          "mako"
          "scootnotify"
        ];
        default = "scootnotify";
      };
      options.programs.scoot.desktop.notifications.settings = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
      };
      config.programs.scoot.desktop = {
        inherit enable;
        theme.targets.notifications.enable = lookTarget;
        notifications = {
          enable = notifications;
          inherit daemon settings;
        };
      };
    };

  # A stand-in package that records the arguments `.override` was given.
  fakeNotify = pkgs.runCommand "fake-scootnotify" { } ''
    mkdir -p $out/bin
    printf '#!/bin/sh\n' > $out/bin/scootnotify
    chmod +x $out/bin/scootnotify
  '';
  recorder = fakeNotify // {
    override = args: fakeNotify // { overrideArgs = args; };
  };

  evalWith =
    { side, extra }:
    cfg:
    lib.evalModules {
      modules = [
        (if side == "home" then homeModule else nixosModule)
        baseStubs
        (if side == "home" then homeStubs else nixosStubs)
        {
          programs.scootnotify = cfg // {
            package = cfg.package or scootnotify;
          };
        }
      ]
      ++ extra;
      specialArgs = { inherit pkgs; };
    };
  evalHome = evalWith {
    side = "home";
    extra = [ (profileStub { }) ];
  };
  evalHomeProfile =
    stub:
    evalWith {
      side = "home";
      extra = [ (profileStub stub) ];
    };
  evalNixos = evalWith {
    side = "nixos";
    extra = [ (profileStub { }) ];
  };
  evalNixosProfile =
    stub:
    evalWith {
      side = "nixos";
      extra = [ (profileStub stub) ];
    };

  settingsOf = e: e.config.programs.scootnotify.settings;
  failing = e: map (a: a.message) (lib.filter (a: !a.assertion) e.config.assertions);

  # --- the evaluations ---
  plain = evalHome { enable = true; };
  userWins = evalHome {
    enable = true;
    settings = {
      max_visible = 3;
      dnd_allows_critical = false;
    };
  };
  oneKey = evalHome {
    enable = true;
    settings.max_visible = 8;
  };
  recorded = evalHome {
    enable = true;
    package = recorder;
    features = [ ];
  };
  unrecordable = evalHome {
    enable = true;
    package = fakeNotify;
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
  # The profile default (daemon scootnotify with the slot on):
  # the daemon enables at `mkDefault`.
  optIn = evalHomeProfile {
    enable = true;
    daemon = "scootnotify";
  } { };
  optInDisabled =
    evalHomeProfile
      {
        enable = true;
        daemon = "scootnotify";
      }
      {
        enable = false;
      };
  optInSlotOff = evalHomeProfile {
    enable = true;
    notifications = false;
    daemon = "scootnotify";
  } { };
  optInProfileOff = evalHomeProfile { } { };
  # The mako escape hatch: an explicit `daemon = "mako"` with the
  # profile on leaves this daemon alone (no auto-enable; an explicit
  # `programs.scootnotify.enable` still wins, the way an explicit
  # disable wins over the profile default).
  makoLeavesAlone = evalHomeProfile {
    enable = true;
    daemon = "mako";
  } { };
  osOptIn = evalNixosProfile {
    enable = true;
    daemon = "scootnotify";
  } { };
  # A mako-shaped settings table beside the new daemon: warns, naming
  # the new home for it. Beside mako (or empty) it stays quiet.
  optInWarn = evalHomeProfile {
    enable = true;
    daemon = "scootnotify";
    settings = {
      anchor = "bottom-right";
    };
  } { enable = true; };
  makoSettingsQuiet = evalHomeProfile {
    enable = true;
    daemon = "mako";
    settings = {
      anchor = "bottom-right";
    };
  } { enable = true; };

  # The look target off beside the profile-managed daemon: the file
  # carries `look = false`. Left on (the default) it does not; a mako
  # daemon is not this module's business; an explicit user value wins.
  lookOff = evalHomeProfile {
    enable = true;
    lookTarget = false;
  } { };
  lookOffNixos = evalNixosProfile {
    enable = true;
    lookTarget = false;
  } { };
  lookOn = evalHomeProfile { enable = true; } { };
  lookOffMako = evalHomeProfile {
    enable = true;
    daemon = "mako";
    lookTarget = false;
  } { enable = true; };
  lookOffUserOn = evalHomeProfile {
    enable = true;
    lookTarget = false;
  } { settings.look = true; };
  lookOffProfileOff = evalHomeProfile { lookTarget = false; } { };

  # Rendered files the real binary checks: every configuration that
  # renders one, plus a hand-made one it must refuse.
  rendered = {
    plain = plain.config.programs.scootnotify.configFile;
    user-wins = userWins.config.programs.scootnotify.configFile;
    one-key = oneKey.config.programs.scootnotify.configFile;
    nixos = os.config.programs.scootnotify.configFile;
    look-off = lookOff.config.programs.scootnotify.configFile;
    look-off-nixos = lookOffNixos.config.programs.scootnotify.configFile;
  };
  unknownKey = tomlFormat.generate "unknown-key.toml" { colors.backgroun = "#101010"; };

  homeSvc = plain.config.systemd.user.services.scootnotify;
  osSvc = os.config.systemd.user.services.scootnotify;
  # The activation file's store path: read at BUILD time in the shell
  # below, not at eval (`builtins.readFile` would realize it during
  # evaluation and break platforms without a builder).
  serviceFile =
    plain.config.xdg.dataFile."dbus-1/services/org.freedesktop.Notifications.service".source;

  is = a: b: a == b;
  checks = {
    "empty settings render an empty file, without a look layer" =
      settingsOf plain == { } && !(settingsOf plain ? look);
    "a user's settings render verbatim, and nothing else" =
      settingsOf userWins == {
        max_visible = 3;
        dnd_allows_critical = false;
      };
    "a user's one key renders beside nothing else" =
      settingsOf oneKey == {
        max_visible = 8;
      };
    "features reaches the build as no-default-features + the list" =
      recorded.config.programs.scootnotify.finalPackage.overrideArgs == {
        buildNoDefaultFeatures = true;
        buildFeatures = [ ];
      };
    "features null leaves the package untouched" =
      plain.config.programs.scootnotify.finalPackage == scootnotify;
    "features with a package that has no .override is refused by name" =
      lib.any (lib.hasInfix "programs.scootnotify.features needs") (failing unrecordable);
    "enable = false manages nothing" =
      off.config.home.packages == [ ]
      && off.config.xdg.configFile == { }
      && off.config.xdg.dataFile == { }
      && osOff.config.environment.etc == { }
      && osOff.config.services.dbus.packages == [ ];
    "systemd.enable = false keeps the package and the file, not the unit" =
      !(offUnit.config.systemd.user.services ? scootnotify)
      && offUnit.config.xdg.configFile ? "scoot/notify.toml"
      && !(osNoUnit.config.systemd.user.services ? scootnotify)
      && osNoUnit.config.environment.etc ? "scoot/notify.toml";
    "a null package with enable is refused by name" =
      lib.any (lib.hasInfix "programs.scootnotify.package is null") (failing nullPkg);
    "home: package installed, file at the daemon's default path" =
      plain.config.home.packages == [ scootnotify ]
      &&
        plain.config.xdg.configFile."scoot/notify.toml".source
        == plain.config.programs.scootnotify.configFile;
    "home: the D-Bus activation file is installed beside the unit" =
      plain.config.xdg.dataFile ? "dbus-1/services/org.freedesktop.Notifications.service";
    "home: the unit is Type=dbus owning the Notifications name" =
      homeSvc.Service.Type == "dbus" && homeSvc.Service.BusName == "org.freedesktop.Notifications";
    "home: ExecStart runs the daemon, ExecReload re-themes it in place" =
      lib.hasInfix "/bin/scootnotify daemon" homeSvc.Service.ExecStart
      && lib.hasInfix "/bin/scootnotify msg reload" homeSvc.Service.ExecReload;
    "home: a taken name stops the unit instead of restart-looping" =
      homeSvc.Service.RestartPreventExitStatus == 3;
    "home: a new config reloads, never restarts" =
      homeSvc.Unit.X-Reload-Triggers == [ "${plain.config.programs.scootnotify.configFile}" ]
      && !(homeSvc.Unit ? X-Restart-Triggers);
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
    "nixos: package and an /etc file naming that file" =
      os.config.environment.systemPackages == [ scootnotify ]
      &&
        os.config.environment.etc."scoot/notify.toml".source == os.config.programs.scootnotify.configFile;
    "nixos: the activation package is published on the session bus" =
      os.config.services.dbus.packages != [ ];
    "nixos: the unit reloads on a config change, never restarts" =
      osSvc.reloadTriggers == [ os.config.programs.scootnotify.configFile ]
      && osSvc.serviceConfig.ExecReload == "${scootnotify}/bin/scootnotify msg reload"
      && osSvc.serviceConfig.RestartPreventExitStatus == 3;
    "no Stylix switch: Stylix reaches it only as the stylix look" =
      !(settingsOf plain ? stylix) && !(plain.config.programs.scootnotify ? stylix);
    "the profile default (scootnotify) enables this daemon" = optIn.config.programs.scootnotify.enable;
    "with the profile off the daemon stays off" =
      !(optInProfileOff.config.programs.scootnotify.enable or false) && settingsOf optInProfileOff == { };
    "the mako escape hatch leaves this daemon alone" =
      !(makoLeavesAlone.config.programs.scootnotify.enable or false);
    "the profile default enables this daemon at mkDefault" = optIn.config.programs.scootnotify.enable;
    "an explicit disable still wins over the profile default" =
      !optInDisabled.config.programs.scootnotify.enable;
    "the profile default needs the notifications slot on" =
      !(optInSlotOff.config.programs.scootnotify.enable or false);
    "the profile default enables the NixOS half too" = osOptIn.config.programs.scootnotify.enable;
    "a mako-shaped settings table beside the new daemon warns, naming the new home" =
      lib.any (lib.hasInfix "programs.scootnotify.settings") optInWarn.config.warnings;
    "the same table beside mako stays quiet" = makoSettingsQuiet.config.warnings == [ ];
    "the look target off writes look = false into the daemon's file" =
      settingsOf lookOff == { look = false; } && settingsOf lookOffNixos == { look = false; };
    "the look target on (the default) leaves the file without a look key" = settingsOf lookOn == { };
    "the look target off enables the daemon as before, with no warning" =
      lookOff.config.programs.scootnotify.enable && lookOff.config.warnings == [ ];
    "the look target off beside mako leaves this daemon's file alone" = settingsOf lookOffMako == { };
    "an explicit settings.look wins over the target" = settingsOf lookOffUserOn == { look = true; };
    "the look target off with the profile off changes nothing" = settingsOf lookOffProfileOff == { };
    "an empty table beside the new daemon stays quiet" = optIn.config.warnings == [ ];
  };
  failed = lib.attrNames (lib.filterAttrs (_: ok: !ok) checks);

  # Text the shell then compares with what tomllib read back.
  expectations =
    lib.mapAttrsToList
      (name: e: {
        inherit name;
        file = e.config.programs.scootnotify.configFile;
        settings = e.config.programs.scootnotify.settings;
      })
      {
        inherit
          plain
          userWins
          oneKey
          lookOff
          ;
      };
in
assert lib.assertMsg (failed == [ ]) "scootnotify module checks failed: ${builtins.toJSON failed}";
runCommand "scootnotify-modules"
  {
    nativeBuildInputs = [ python3 ];
    inherit scootnotify;
    expectations = builtins.toJSON (
      map (x: {
        inherit (x) name settings;
        file = "${x.file}";
      }) expectations
    );
    passAsFile = [ "expectations" ];
    inherit unknownKey;
    inherit serviceFile;
    files = lib.concatStringsSep " " (lib.mapAttrsToList (n: f: "${n}=${f}") rendered);
    checkNames = builtins.toJSON (lib.attrNames checks);
  }
  ''
    # 1. Every rendered file parses, and reads back as exactly the
    #    user's settings (verbatim: no look layer, no extra keys).
    python3 - <<'PY'
    import json, os, tomllib
    for x in json.load(open(os.environ["expectationsPath"])):
        got = tomllib.load(open(x["file"], "rb"))
        assert got == x["settings"], (x["name"], got, x["settings"])
        print("toml ok:", x["name"])
    PY

    # 1b. The D-Bus activation file names the same binary the unit runs
    #     with `SystemdService=scootnotify.service` (read at build time:
    #     reading it at eval would realize it during evaluation).
    grep -F -q "Name=org.freedesktop.Notifications" "$serviceFile"
    grep -F -q "/bin/scootnotify daemon" "$serviceFile"
    grep -F -q "SystemdService=scootnotify.service" "$serviceFile"
    echo "activation file ok: $serviceFile"

    # 2. The real binary, `daemon --check` over each rendered file (its
    #    output goes in `got`, not `out`, which is $out): what a start
    #    does before it owns the name, with no compositor to fake. An
    #    empty runtime dir and no `WAYLAND_DISPLAY`: --check touches
    #    neither, and the check would catch it if that changed.
    export XDG_RUNTIME_DIR=$TMPDIR/run HOME=$TMPDIR/home
    mkdir -p "$XDG_RUNTIME_DIR" "$HOME"
    unset WAYLAND_DISPLAY
    for pair in $files; do
      name=''${pair%%=*}; file=''${pair#*=}
      status=0
      $scootnotify/bin/scootnotify daemon --check --config "$file" >$TMPDIR/got 2>$TMPDIR/err || status=$?
      if [ "$status" != 0 ]; then
        echo "scootnotify refused the rendered $name ($file), exit $status:"; cat $TMPDIR/err; cat "$file"; exit 1
      fi
      echo "scootnotify accepted: $name"
    done
    [ -z "$(ls -A "$XDG_RUNTIME_DIR")" ] || { echo "--check left files in the runtime dir:"; ls -A "$XDG_RUNTIME_DIR"; exit 1; }
    # 3. A file with an unknown key is refused, exit 1: the control
    #    that shows the check can fail.
    status=0
    $scootnotify/bin/scootnotify daemon --check --config "$unknownKey" >$TMPDIR/got 2>$TMPDIR/err || status=$?
    if [ "$status" != 1 ]; then
      echo "control failed: the unknown key was not refused with exit 1 (exit $status):"; cat $TMPDIR/err; exit 1
    fi
    echo "scootnotify refused the unknown key, as it must"

    mkdir -p $out
    echo "$checkNames" > $out/evaluated-checks.json
  ''
