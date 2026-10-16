# `programs.scoot.desktop.nightlight` for home-manager: the night-light
# user unit (wlsunset or gammastep over `wlr-gamma-control-v1`). The
# option shapes live in `./desktop.nix` (shared with the NixOS side);
# the `package` default lives here because only this side has `pkgs`
# (defaulting to the daemon's own tool). See
# site/src/content/docs/desktop/index.md#night-light ("Night light").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  night = cfg.desktop.nightlight;

  # The desktop profile's shared option subtree and the one helper
  # that reads look-derived values (never hand-mapped here).
  themeLook = import ./theme-look.nix { inherit lib; };
  look = themeLook.lookFor cfg;
  themed = themeLook.themed cfg "nightlight";

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # Location mode when the pair is set (the manual sunrise/sunset and
  # the duration then go unread: wlsunset's duration applies to manual
  # times only, and the sun computes the boundaries instead).
  located = night.latitude != null && night.longitude != null;

  # A null beside `enable` is the loud assertion below, not a throw
  # inside `getExe`: the same guard the idle policy uses, since
  # standalone evals collect assertions without enforcing them.
  toolsReady = night.package != null;

  # The night temperature comes from the live look at unit start only
  # when the look's own warmth is in effect: a look is set, its
  # nightlight target is on, and no explicit `nightTemp` overrides it
  # (your own setting wins, so the wrapper then uses it directly and
  # never reads the fragment). Baked at evaluation: the two wrappers
  # below differ, there is no runtime gate to drift. `scoot look apply`
  # restarts this unit on every swap, so the new temperature lands then.
  # Every other option stays baked at evaluation (a unit rewrite still
  # restarts the daemon, as before).
  useLive = themed && look != null && night.nightTemp == look.nightTemp;

  # The daemon is fixed at evaluation (a unit rewrite restarts the
  # daemon on any option change), so only the taken branch renders:
  # no dead flags, and a negative longitude rides as its own word
  # (`-L -121.9`: getopt takes the next word verbatim).
  daemonExec =
    if night.daemon == "gammastep" then
      ''exec ${lib.getExe night.package} -m wayland -t "${toString night.dayTemp}:$temp" -l "${toString night.latitude}:${toString night.longitude}" -g "${toString night.gamma}:${toString night.gamma}:${toString night.gamma}"''
    else if located then
      ''exec ${lib.getExe night.package} -T ${toString night.dayTemp} -t "$temp" -l ${toString night.latitude} -L ${toString night.longitude} -g ${toString night.gamma}''
    else
      ''exec ${lib.getExe night.package} -T ${toString night.dayTemp} -t "$temp" -S ${night.sunrise} -s ${night.sunset} -d ${toString night.duration} -g ${toString night.gamma}'';
  nightlightRun =
    if useLive then
      pkgs.writeShellScriptBin "scoot-nightlight-run" ''
        look="''${XDG_STATE_HOME:-$HOME/.local/state}/scoot/look/nightlight"
        temp="$(sed -n 's/^temperature=//p' "$look" 2>/dev/null | head -n 1)"
        case "$temp" in
          "" | *[!0-9]* | ??????*) temp=${toString night.nightTemp} ;;
        esac
        if [ "$temp" -lt 1000 ] || [ "$temp" -gt 10000 ]; then
          temp=${toString night.nightTemp}
        fi
        ${daemonExec}
      ''
    else
      pkgs.writeShellScriptBin "scoot-nightlight-run" ''
        # Static: an explicit nightTemp wins, or no look theme applies;
        # the live fragment stays unread even after `scoot look apply`.
        temp=${toString night.nightTemp}
        ${daemonExec}
      '';

  # `builtins.match` needs the whole value, so each pattern below is a
  # full `HH:MM` (00:00 to 23:59) and nothing else.
  isClock = value: builtins.match "^([01][0-9]|2[0-3]):[0-5][0-9]$" value != null;
in
{
  # Seeds the live look link before the first daemon start (the
  # leaf fragments below point at it; a dangling link refuses
  # mako and wob at startup). One shared module: importing it
  # four times evaluates once.
  imports = [ ./look-leaves-seed.nix ];
  options.programs.scoot.desktop.nightlight = {
    # The tool below is Linux-only: its attributes refuse evaluation
    # when forced on Darwin, so `or null` alone does not save them (the
    # Darwin `nix flake check` run reads every default). Off Linux it
    # defaults to null, which the assertion below refuses loudly
    # instead of installing nothing silently. Follows the daemon: the
    # warming tool for `wlsunset`, the sunrise/sunset one for
    # `gammastep` (point it at your own build of either; the unit
    # calls it with that daemon's flags, so the two must agree).
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default =
        if !isLinux then
          null
        else if night.daemon == "gammastep" then
          (if pkgs ? gammastep then pkgs.gammastep else null)
        else
          (if pkgs ? wlsunset then pkgs.wlsunset else null);
      defaultText = lib.literalExpression ''
        if pkgs.stdenv.hostPlatform.isLinux then (wlsunset for daemon "wlsunset", gammastep for "gammastep") else null
      '';
      description = ''
        The night-light tool to run (must speak the daemon's flags:
        wlsunset's for `daemon = "wlsunset"`, gammastep's for
        `daemon = "gammastep"`). Null installs nothing. Linux-only:
        null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The daemon: its package on PATH, and the unit that runs it.
    # Wanted by `scoot-session.target` -- scoot's own session scope
    # (started by the launcher past the display import, so the display
    # is there when the daemon starts) -- never the shared
    # `graphical-session.target`, which every other desktop reaches too
    # and would start this daemon inside someone else's session.
    # Retried like the bar's unit rather than conditioned (a skipped
    # start is never retried). No restart trigger: every option lands
    # in the wrapper below, so any change rewrites the unit and
    # restarts it; a look swap restarts it through
    # `systemctl --user try-restart` from `scoot look apply` instead.
    (lib.mkIf night.enable {
      assertions = [
        {
          assertion = night.package != null;
          message = ''
            programs.scoot.desktop.nightlight.enable is set but
            programs.scoot.desktop.nightlight.package is null: set it
            explicitly (apply the overlay, or point at a wlsunset for
            daemon "wlsunset" or a gammastep for "gammastep").
          '';
        }
        {
          assertion = night.dayTemp >= 1000 && night.dayTemp <= 10000;
          message = ''
            programs.scoot.desktop.nightlight.dayTemp is ${toString night.dayTemp}:
            the day color temperature is 1000 to 10000 Kelvin.
          '';
        }
        {
          assertion = night.nightTemp >= 1000 && night.nightTemp <= 10000;
          message = ''
            programs.scoot.desktop.nightlight.nightTemp is ${toString night.nightTemp}:
            the night color temperature is 1000 to 10000 Kelvin.
          '';
        }
        {
          # Strictly below: wlsunset exits at once when the two are equal,
          # and Restart=on-failure would re-exec it every 2 s forever.
          assertion = night.nightTemp < night.dayTemp;
          message = ''
            programs.scoot.desktop.nightlight.nightTemp is ${toString night.nightTemp},
            not below dayTemp ${toString night.dayTemp}: the night is warmer
            (lower) than the day. For no warming, set
            desktop.nightlight.enable = false instead.
          '';
        }
        {
          assertion = isClock night.sunrise;
          message = ''
            programs.scoot.desktop.nightlight.sunrise is "${night.sunrise}":
            the manual sunrise is 24-hour `HH:MM` (00:00 to 23:59).
          '';
        }
        {
          assertion = isClock night.sunset;
          message = ''
            programs.scoot.desktop.nightlight.sunset is "${night.sunset}":
            the manual sunset is 24-hour `HH:MM` (00:00 to 23:59).
          '';
        }
        {
          assertion = night.duration >= 0 && night.duration <= 7200;
          message = ''
            programs.scoot.desktop.nightlight.duration is ${toString night.duration}:
            the transition is 0 (snap) to 7200 seconds (2 h).
          '';
        }
        {
          assertion = night.gamma >= 0.1 && night.gamma <= 10;
          message = ''
            programs.scoot.desktop.nightlight.gamma is ${toString night.gamma}:
            the multiplier is 0.1 to 10 (1.0 is neutral).
          '';
        }
        {
          assertion = night.latitude == null || (night.latitude >= -90 && night.latitude <= 90);
          message = ''
            programs.scoot.desktop.nightlight.latitude is ${toString night.latitude}:
            decimal degrees, -90 to 90.
          '';
        }
        {
          assertion = night.longitude == null || (night.longitude >= -180 && night.longitude <= 180);
          message = ''
            programs.scoot.desktop.nightlight.longitude is ${toString night.longitude}:
            decimal degrees, -180 to 180.
          '';
        }
        {
          # One coordinate without the other computes nothing: wlsunset
          # would warm on it half-read, gammastep would refuse it.
          assertion = (night.latitude == null) == (night.longitude == null);
          message = ''
            programs.scoot.desktop.nightlight sets only one of
            latitude/longitude: set both for location mode, or neither
            for the manual schedule.
          '';
        }
        {
          # Geoclue is not wired (it needs a system service and the
          # network behind it), so a gammastep with nowhere to stand
          # would reach for it and fail at runtime -- refuse it here
          # instead, naming the pair.
          assertion = night.daemon != "gammastep" || located;
          message = ''
            programs.scoot.desktop.nightlight.daemon is "gammastep" with
            no latitude/longitude: gammastep always needs a location
            (geoclue is not wired), so set both -- or stay on
            daemon "wlsunset", whose manual schedule needs none.
          '';
        }
      ];

      home.packages = lib.optional (night.package != null) night.package;

      systemd.user.services.scoot-nightlight = lib.mkIf toolsReady {
        Unit = {
          Description = "scoot night light (gamma ramp over wlr-gamma-control)";
          PartOf = [ "scoot-session.target" ];
          After = [ "scoot-session.target" ];
          # Unending retries (`StartLimitIntervalSec` lives in `[Unit]`:
          # systemd ignores it in `[Service]`).
          StartLimitIntervalSec = 0;
        };
        Service = {
          ExecStart = "${nightlightRun}/bin/scoot-nightlight-run";
          Restart = "on-failure";
          RestartSec = 2;
        };
        Install.WantedBy = [ "scoot-session.target" ];
      };
    })

    # The look's own night warmth, at `mkDefault` so a value the user
    # wrote wins (a plain definition beats any default; the same
    # precedence the profile's own `mkDefault`s use). `mkOptionDefault`
    # would collide with the option's declared default (same priority),
    # so the look rides one level up instead.
    (lib.mkIf themed {
      programs.scoot.desktop.nightlight.nightTemp = lib.mkDefault look.nightTemp;
    })

    # The profile turns the slot on (still individually disable-able
    # at plain priority, the way the clipboard slot works).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.nightlight.enable = lib.mkDefault true;
    })
  ];
}
