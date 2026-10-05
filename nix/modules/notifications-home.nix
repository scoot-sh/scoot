# `programs.scoot.desktop.notifications` for home-manager: the mako
# user unit, its config, and the bar feed (DND state and unread count
# into the bar's `push` module). The option shapes live in
# `./desktop.nix` (shared with the NixOS side); the `package` default
# lives here because only this side has `pkgs`. See docs/nix.md
# ("Notifications").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  notif = cfg.desktop.notifications;

  # The desktop profile's shared option subtree and look palettes (the
  # `enum` type guarantees the name, so the lookup cannot fail).
  desktop = import ./desktop.nix { inherit lib; };
  look = if cfg.desktop.look == null then null else desktop.looks.${cfg.desktop.look};
  themed = look != null && (cfg.desktop.theme.targets.notifications.enable or true);

  # The bar's build, when its module is imported beside this one (the
  # `or {}` keeps this evaluating without it, the way `scootbar.nix`
  # reads the profile). The feed calls its CLI by absolute path then;
  # otherwise by name from PATH, tolerating a missing bar.
  barBin = (config.programs.scootbar or { }).finalPackage or null;
  scootbarBin = if barBin != null then lib.getExe barBin else "scootbar";

  # The feed: DND state (`makoctl mode`) and the unread count
  # (`makoctl list`, urgent when any is critical) into the bar's
  # `push` module -- the half the `scootnotify` pointer entry promises,
  # so swapping the daemon later changes nothing the user configured.
  # `--watch` does an initial sync, then re-syncs on mako's own bus
  # signals, one JSON object per line (`busctl monitor --json=short`;
  # the two `--match` rules are OR'd, verified against systemd 261):
  # `PropertiesChanged` on mako's path (arrivals, dismissals,
  # timeouts and mode changes all emit one: `Notifications` and
  # `Modes` carry `EMITS_INVALIDATION` in mako v1.11.0's
  # `dbus/mako.c`) and `NameOwnerChanged` for the Notifications name
  # (a mako restart emits no `PropertiesChanged`, so without the
  # second match the feed would show stale DND/count until the next
  # event; losing the name clears the bar instead of leaving the
  # stale state). No polling, and no `on-notify` hook, so every mako
  # key stays overridable in `settings` without breaking the feed.
  bridge = pkgs.writeShellApplication {
    name = "scoot-notify-sync";
    runtimeInputs = [
      notif.package
      pkgs.jq
      pkgs.systemdMinimal # for busctl
      pkgs.coreutils # for stdbuf
    ]
    ++ lib.optional (barBin != null) barBin;
    text = ''
      # Pushes $1 into the bar, saying nothing either way. Returns 0
      # on success, 1 with the daemon's refusal in `_push_err`.
      # Capturing the refusal (instead of letting the CLI print it)
      # is what keeps each failed sync to exactly one stderr line.
      _push_err=""
      push_payload() {
          _push_err="$(${scootbarBin} msg set notifications "$1" 2>&1)" && return 0
          return 1
      }

      # The one stderr line for a failed push, naming the real cause:
      # a module that is not placed (the user never added the
      # one-liner -- the bar is running, it just shows no such
      # module) vs a bar that is not running at all. Distinguishing
      # the two is the point: the old message blamed the bar for a
      # module the user had not placed yet.
      report_push_failure() {
          case "$_push_err" in
              *"is not placed"*)
                  echo "scoot-notify-sync: the bar shows no notifications module (add \"notifications\" to a bar list in programs.scootbar.settings -- see docs/nix.md \"Notifications\"); leaving it" >&2
                  ;;
              *)
                  echo "scoot-notify-sync: cannot reach the bar (''${_push_err:-the bar is not running}); leaving it" >&2
                  ;;
          esac
      }

      # mako is gone (its modes unreadable, or the bus name lost its
      # owner): clear the bar rather than leaving the stale DND/count
      # up. Exactly one stderr line either way -- a cleared bar says
      # so, a failed clear reports why the push failed, never both.
      mako_gone() {
          if push_payload '{"text":""}'; then
              echo "scoot-notify-sync: mako is not running, cleared the bar" >&2
          else
              report_push_failure
          fi
      }

      sync_now() {
          modes="$(makoctl mode 2>/dev/null)" || {
              mako_gone
              return 0
          }
          # One mode per line (or space-separated): normalize so the
          # match below needs no assumption about the separator.
          flat=" $(printf '%s' "$modes" | tr '[:space:]' ' ') "
          dnd=0
          case "$flat" in
              *" do-not-disturb "*) dnd=1 ;;
          esac
          list="$(makoctl list -j 2>/dev/null || printf '%s' '[]')"
          # A malformed list (never seen from mako itself) counts as
          # empty, never as a crash: under `errexit` an unguarded
          # `jq` here would exit the script, and
          # `Restart=on-failure` would respawn it into the same
          # failure every 2 s. One warning line, then the empty
          # state.
          bad_list=0
          n="$(printf '%s' "$list" | jq 'length' 2>/dev/null)" || { n=""; bad_list=1; }
          case "$n" in
              '''|*[!0-9]*) n=0; bad_list=1 ;;
          esac
          crit="$(printf '%s' "$list" | jq '[.[] | select(.urgency == 2)] | length' 2>/dev/null)" || { crit=""; bad_list=1; }
          case "$crit" in
              '''|*[!0-9]*) crit=0; bad_list=1 ;;
          esac
          if [ "$bad_list" = 1 ]; then
              echo "scoot-notify-sync: makoctl list printed malformed JSON; showing empty" >&2
          fi
          # Compact (`-c`): the payload travels as one `msg set`
          # argument, and stays one line wherever it is logged.
          payload="$(jq -c -n --argjson n "$n" --argjson dnd "$dnd" --argjson crit "$crit" \
              'if $dnd == 1 then
                  {text: (if $n > 0 then "DND \($n)" else "DND" end),
                   class: "muted",
                   tooltip: (if $n > 0 then "\($n) notifications held by do-not-disturb -- click to let them through" else "do-not-disturb is on -- click to let notifications through" end)}
               elif $n > 0 then
                  {text: "\($n)",
                   class: (if $crit > 0 then "urgent" else "normal" end),
                   tooltip: "\($n) notifications -- click to hold them with do-not-disturb"}
               else {text: ""} end')" || {
              echo "scoot-notify-sync: cannot build the bar payload; leaving the bar as it is" >&2
              return 0
          }
          [ -n "$payload" ] || return 0
          push_payload "$payload" || report_push_failure
      }

      if [ "''${1-}" = "--watch" ]; then
          sync_now
          # The matches filter server-side: only mako's own path and
          # the Notifications name wake this up. A re-sync is
          # idempotent, so a stray signal costs one cheap query, not
          # correctness. A monitor line that is not JSON (or names no
          # member) is skipped, never fatal. The `|| [ -n ... ]`
          # keeps a final line without its trailing newline (a
          # monitor cut mid-write, never the daemon itself, which
          # always terminates its lines).
          stdbuf -o0 -e0 busctl --user monitor --json=short --match "type='signal',interface='org.freedesktop.DBus.Properties',path='/fr/emersion/Mako'" --match "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'" 2>/dev/null | while IFS= read -r line || [ -n "$line" ]; do
              member="$(printf '%s' "$line" | jq -r '.member // empty' 2>/dev/null)" || continue
              [ -n "$member" ] || continue
              case "$member" in
                  PropertiesChanged)
                      path="$(printf '%s' "$line" | jq -r '.path // empty' 2>/dev/null)" || continue
                      [ "$path" = "/fr/emersion/Mako" ] && sync_now
                      ;;
                  NameOwnerChanged)
                      name="$(printf '%s' "$line" | jq -r '.payload.data[0] // empty' 2>/dev/null)" || continue
                      [ "$name" = "org.freedesktop.Notifications" ] || continue
                      new="$(printf '%s' "$line" | jq -r '.payload.data[2] // empty' 2>/dev/null)" || continue
                      if [ -n "$new" ]; then
                          sync_now
                      else
                          mako_gone
                      fi
                      ;;
              esac
          done
          echo "scoot-notify-sync: bus monitor ended" >&2
          exit 1
      else
          sync_now
      fi
    '';
  };

  # The generated mako config: the `overlay` layer first (mako's own
  # default is `top`, which the compositor hides under fullscreen
  # windows -- `docs/protocols.md` "Fullscreen"), then the look's
  # roles as mako leaves when themed, then the two generated sections.
  # `settings` wins per key over the globals; the sections always
  # render (only global keys are overridable there).
  makoConfig = pkgs.writeText "mako-config" (
    lib.concatStringsSep "\n" (
      [ "# Generated by programs.scoot.desktop.notifications -- see docs/nix.md." ]
      ++ lib.mapAttrsToList (name: value: "${name}=${value}") (
        (
          {
            layer = "overlay";
          }
          // lib.optionalAttrs themed {
            background-color = look.barColors.background;
            text-color = look.barColors.foreground;
            border-color = look.appearance.focus_ring_active_color;
            progress-color = "over ${look.barColors.accent}";
          }
        )
        // notif.settings
      )
      ++ [
        "[mode=do-not-disturb]"
        "invisible=1"
      ]
      ++ lib.optionals themed [
        "[urgency=critical]"
        "border-color=${look.barColors.urgent}"
      ]
    )
    + "\n"
  );

  # A null beside `enable` is the loud assertion below, not a throw
  # inside `getExe`: the same guard the idle policy uses, since
  # standalone evals collect assertions without enforcing them.
  toolsReady = notif.package != null;
  # The lean mako (no GTK stack -- see `notifications-mako.nix`),
  # shared with the NixOS side's default so either side alone names
  # the same daemon. Guarded off Linux like the stock attribute it
  # wraps: `pkgs.mako` refuses evaluation there.
  leanMako = import ./notifications-mako.nix { inherit pkgs; };
in
{
  options.programs.scoot.desktop.notifications = {
    # The tool below is Linux-only: its attribute exists on Darwin
    # but refuses evaluation when forced, so `or null` alone does not
    # save it (the Darwin `nix flake check` run reads every default).
    # Off Linux it defaults to null, which the assertion below
    # refuses loudly instead of installing nothing silently.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default =
        if pkgs.stdenv.hostPlatform.isLinux then (if pkgs ? mako then leanMako else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then leanMako else null";
      description = ''
        The mako package to run the notification daemon from (must
        speak the daemon's flags and `makoctl`). Defaults to a lean
        mako without the GTK stack (see `notifications-mako.nix`).
        Null installs nothing. Linux-only: null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The daemon: its package, its config file, and the unit that runs
    # it. Wanted by the graphical session (which the launcher reaches
    # past the display import, so the display is there when mako
    # starts), retried like the bar's unit rather than conditioned
    # (a skipped start is never retried). Named `mako.service`, after
    # the daemon: mako ships a D-Bus activation file for
    # `org.freedesktop.Notifications` routing through
    # `SystemdService=mako.service`, so a `Notify` with the daemon
    # down starts this unit instead of failing -- and the `ExecStart`
    # below is what it runs.
    (lib.mkIf notif.enable {
      assertions = [
        {
          assertion = notif.package != null;
          message = ''
            programs.scoot.desktop.notifications.enable is set but
            programs.scoot.desktop.notifications.package is null: set
            it explicitly (apply the overlay, or point at a mako).
          '';
        }
      ];

      # The bridge runs from its store path in the feed unit below,
      # not from PATH: installing it would put a second copy of the
      # package set's shape into every profile user's profile for no
      # runtime need (a manual sync is `systemctl --user restart
      # scoot-notify-sync`).
      home.packages = lib.optional (notif.package != null) notif.package;

      xdg.configFile."mako/config".source = lib.mkIf toolsReady makoConfig;

      systemd.user.services.mako = lib.mkIf toolsReady {
        Unit = {
          Description = "mako notification daemon (desktop.notifications)";
          PartOf = [ "graphical-session.target" ];
          After = [ "graphical-session.target" ];
          # A new config restarts the daemon (it starts in
          # milliseconds; `makoctl reload` would do, but a restart is
          # what the bar and idle units do).
          X-Restart-Triggers = [ "${makoConfig}" ];
        };
        Service = {
          # D-Bus activation (`Type=dbus` plus the name): the unit is
          # started when claimed, and considered started once mako
          # owns the name.
          Type = "dbus";
          BusName = "org.freedesktop.Notifications";
          # Activation can arrive before the session reaches the
          # graphical target (a `Notify` in an early autostart): skip
          # cleanly then -- no restart -- and the next `Notify`
          # re-activates. The wanted-by below starts it with the
          # display in the common case.
          ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
          ExecStart = "${lib.getExe notif.package}";
          ExecReload = "${lib.getExe' notif.package "makoctl"} reload";
          Restart = "on-failure";
          RestartSec = 2;
          # Unending retries, like the bar's unit: a start before the
          # compositor is up must retry, not die at the burst limit (a
          # broken config then logs every 2 s until fixed -- loud beats
          # silent).
          StartLimitIntervalSec = 0;
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };
    })

    # The bar feed beside the daemon: an initial sync, then a sync per
    # mako bus signal. Kept in its own element (and its own unit): the
    # daemon stays up without it, and it stays quiet without the bar.
    (lib.mkIf (notif.enable && toolsReady) {
      systemd.user.services.scoot-notify-sync = {
        Unit = {
          Description = "desktop.notifications bar feed (DND + count into the bar)";
          PartOf = [ "graphical-session.target" ];
          After = [
            "graphical-session.target"
            "mako.service"
          ];
          # A new bridge restarts the feed.
          X-Restart-Triggers = [ "${bridge}" ];
        };
        Service = {
          ExecStart = "${bridge}/bin/scoot-notify-sync --watch";
          Restart = "on-failure";
          RestartSec = 2;
          StartLimitIntervalSec = 0;
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };
    })

    # The profile turns the daemon on (still individually
    # disable-able at plain priority, the way `session.enable` works on
    # the NixOS side).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.notifications.enable = lib.mkDefault true;
    })
  ];
}
