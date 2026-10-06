# `programs.scoot.desktop.displays` for home-manager: scoot-native
# arrangement profiles (kanshi-class matching, without kanshi).
#
# Why not kanshi: stock kanshi runs its `exec` hooks only after the
# compositor answers an output configuration with `succeeded`
# (kanshi 1.9.0 `main.c`: `config_handle_succeeded` runs the commands,
# `config_handle_failed` only logs), and scoot's
# `wlr-output-management-v1` write half answers every configuration
# with `failed` by design (see
# `crates/scoot/src/compositor/output_management.rs`). kanshi matches
# the profile through the read half and then fails it, every hotplug,
# running nothing -- proven live against headless scoot, and its
# event loop (`poll(-1)`, no timers) measured at ~0 idle wakeups.
# Making the protocol writable would be atomic-modeset surgery across
# three backends for a packaging slot (the durable home for that is
# `docs/backlog/resolved/output-management-reconfiguration-done.md`;
# this watcher is the stopgap until then) -- while `subscribe`
# (events), `outputs` (matching), `output-scale` and `output-power`
# (applying, both live runtime state) give matching plus applying over
# scoot's own IPC. So the watcher below speaks that IPC, and wakes on
# hotplug the same way kanshi does (blocking read, no polling).
#
# The watcher never writes the user's config file. Under the desktop
# profile that file is home-manager's read-only store symlink, so a
# watcher that edited it would either fail while reporting success or
# replace the symlink and lose to every rebuild. Scales and power are
# runtime state instead: a successful reload or a restart goes back to
# the config file's scales (a reload leaves power alone; a restart
# starts every output on), and the watcher re-applies at session start,
# on every output event (an add included, so a first plug is heard),
# and on the idle policy's resume -- not on a reload, which no IPC
# event reports (see the site's troubleshooting entry). A home-manager
# rebuild never reloads the session, so it never drops an applied
# scale. The one file it does write is its own record of the outputs
# it powered off (`$XDG_RUNTIME_DIR/scoot-displays.off`), so it can
# undo exactly those and nothing an idle policy blanked.
#
# The option shapes live in `./desktop.nix` (shared with the NixOS
# side); there are no `package` defaults here because the slot installs
# no daemon of its own (the watcher is built below from `jq`, `flock`
# and the core utilities beside the scoot binary). The NixOS side has
# nothing to do: no system service, no backend package. See
# site/src/content/docs/desktop/index.md#displays.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  dis = cfg.desktop.displays;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # Absolute tool paths when the slot knows a package, bare names from
  # PATH otherwise (a direct-module setup without the overlay, or a
  # files-only eval): a missing tool then fails loudly in the script
  # (its own message on stderr, exit 1 -- never wedging anything,
  # since the watcher only calls out), so the unit is always safe to
  # start. Same pattern as the audio slot's `binOr`. Every other tool
  # below is absolute for the same reason systemd units get no useful
  # PATH: `scoot msg` stays overridable (the session's own binary may
  # differ from the module's package), the rest does not move.
  binOr = name: pkg: if pkg != null then "${lib.getExe' pkg name}" else name;

  scootBin = binOr "scoot" cfg.package;
  jqBin = lib.getExe pkgs.jq;
  sleepBin = lib.getExe' pkgs.coreutils "sleep";
  flockBin = lib.getExe' pkgs.util-linux "flock";
  mvBin = lib.getExe' pkgs.coreutils "mv";
  rmBin = lib.getExe' pkgs.coreutils "rm";

  # The profiles, as the watcher matches them: the `outputs` set per
  # profile (exact connected set, connector names), the per-output
  # scale table, and the power-off list (`mode` is gone: see its
  # refusal below). Rendered once here, read
  # by absolute store path (a changed file restarts the unit, so the
  # watcher never runs stale profiles).
  profilesFile = builtins.toFile "scoot-displays.json" (
    builtins.toJSON (map (profile: removeAttrs profile [ "mode" ]) dis.profiles)
  );

  # The watcher: `status` prints the matched profile, `apply` matches
  # once and applies, `watch` applies at start and re-applies on every
  # output event. Matching is by exact connected connector-name set
  # (first profile in list order wins); make and model cannot key a
  # profile (scoot's IPC does not report them -- `outputs` carries
  # names, rects, scales and power only).
  scootDisplays = pkgs.writeShellScriptBin "scoot-displays" ''
    set -u
    PROFILES=${profilesFile}
    SCOOT=${lib.escapeShellArg scootBin}
    JQ=${lib.escapeShellArg jqBin}
    SLEEP=${lib.escapeShellArg sleepBin}
    FLOCK=${lib.escapeShellArg flockBin}
    MV=${lib.escapeShellArg mvBin}
    RM=${lib.escapeShellArg rmBin}
    RUNTIME="''${XDG_RUNTIME_DIR:-''${TMPDIR:-/tmp}}"
    # The outputs this watcher powered off and has not powered back on,
    # as `[{"name":..,"id":..}]`: the only offs it ever undoes. An off
    # made by anything else (the idle policy's screens-off, `wlopm`, a
    # hand `scoot msg output-power`) is not in here, so the watcher
    # never lights a screen it did not darken. Beside the lock, never
    # beside the config (which the watcher never touches). It outlives
    # a watcher restart (a rebuild's new profiles must still undo the
    # old ones' offs) and a compositor restart inside one login (the
    # runtime dir lives until the last session logs out). So a fresh
    # compositor can find an old record: an entry whose name and id
    # match a live output turns that output on at the first apply, even
    # if something else turned it off in between. It only ever lights a
    # screen, never darkens one.
    HELD="$RUNTIME/scoot-displays.off"

    # The live outputs, or a loud refusal (no session: exit 1, so the
    # unit's on-failure retry covers a watcher started too early).
    live_outputs() {
      "$SCOOT" msg outputs || {
        echo "scoot-displays: no session ($SCOOT msg outputs failed)" >&2
        return 1
      }
    }

    connected_names() {
      "$JQ" -c '[.outputs[].name] | sort' <<<"$1"
    }

    # The first profile whose `outputs` is exactly the connected set,
    # as compact JSON, or nothing when none matches.
    match_profile() {
      "$JQ" -c --argjson connected "$1" \
        'first(.[] | select((.outputs | sort) == $connected)) // empty' \
        "$PROFILES"
    }

    do_status() {
      outputs=$(live_outputs) || return 1
      connected=$(connected_names "$outputs")
      profile=$(match_profile "$connected")
      if [ -z "$profile" ]; then
        echo "none (connected: $connected)"
      else
        echo "$("$JQ" -r '.name' <<<"$profile") (connected: $connected)"
      fi
    }

    # One IPC call, named on failure: scoot's own refusal reaches stderr
    # first (`scoot msg` prints it and exits non-zero), then this line
    # says which call of the apply it was.
    call() {
      if ! "$SCOOT" msg "$@" >/dev/null; then
        echo "scoot-displays: \`scoot msg $*\` failed" >&2
        return 1
      fi
    }

    # The held record, or `[]` when there is none (or it is unreadable:
    # a record nobody can parse holds nothing the watcher could undo).
    read_held() {
      if [ -f "$HELD" ] && "$JQ" -e 'type == "array"' "$HELD" >/dev/null 2>&1; then
        "$JQ" -c . "$HELD"
      else
        echo '[]'
      fi
    }

    # Replaces the held record atomically: a temp file in the same
    # directory renamed over it, so a watcher killed mid-write leaves
    # the old record or the new one, never half of either. Called only
    # under the apply lock. Unchanged content writes nothing.
    write_held() {
      if [ "$1" = "$(read_held)" ]; then
        return 0
      fi
      tmp="$HELD.tmp.$$"
      if printf '%s\n' "$1" > "$tmp" && "$MV" -fT "$tmp" "$HELD"; then
        return 0
      fi
      "$RM" -f "$tmp"
      echo "scoot-displays: could not record power state in $HELD" >&2
      return 1
    }

    # Applies the matched profile purely over IPC, never touching a
    # file. Every call runs even after one fails (the rest still
    # converge), and the apply reports "applied" only when all of them
    # succeeded -- otherwise it exits non-zero, each failed call named.
    #
    # Power, in this order, the same rule whether a profile matches or
    # not:
    # 1. every held output still connected under the same id, and not
    #    disabled by the matched profile, is powered back on and
    #    released -- so undocking from a clamshell profile lights the
    #    panel it darkened, matched profile or none. A held output that
    #    left (or came back under a fresh id) is released without a
    #    call: scoot forgets an output's power state on removal, and a
    #    replugged one comes back on;
    # 2. every output the matched profile disables is recorded as held
    #    first, then powered off -- recorded before the call, so a
    #    watcher killed in between never forgets an off it made.
    # Nothing else is powered on: an output someone else turned off
    # (the idle policy's screens-off above all) stays off.
    #
    # Scale: every output of a matched set gets the profile's scale,
    # or (no entry) its live scale reset to the config file's, so a
    # scale from an earlier profile never lingers. With no match every
    # connected output's scale resets. With no profiles at all no scale
    # call is made (scales someone else set stay theirs), but held
    # outputs are still released: removing the profile that darkened a
    # panel must not leave it dark.
    do_apply() {
      outputs=$(live_outputs) || return 1
      connected=$(connected_names "$outputs")
      failed=0
      have_profiles=1
      profile=""
      if [ "$("$JQ" length "$PROFILES")" -eq 0 ]; then
        have_profiles=0
      else
        profile=$(match_profile "$connected")
      fi
      if [ -n "$profile" ]; then
        disabled=$("$JQ" -c '.disabled' <<<"$profile")
      else
        disabled='[]'
      fi

      # 1. Release: held entries still live under the same id.
      held=$(read_held)
      keep=$("$JQ" -c --argjson live "$outputs" --argjson off "$disabled" \
        '[.[] | . as $h | select(any($live.outputs[]; .name == $h.name and .id == $h.id))
              | select($off | index($h.name))]' <<<"$held")
      while IFS=$'\t' read -r out id; do
        [ -n "$out" ] || continue
        if ! call output-power "$id" on; then
          failed=1
          keep=$("$JQ" -c --arg n "$out" --argjson i "$id" '. + [{name: $n, id: $i}]' <<<"$keep")
        fi
      done < <("$JQ" -r --argjson live "$outputs" --argjson off "$disabled" \
        '.[] | . as $h | select(any($live.outputs[]; .name == $h.name and .id == $h.id))
             | select(($off | index($h.name)) | not) | [.name, .id] | @tsv' <<<"$held")
      write_held "$keep" || failed=1

      # 2. Scale.
      if [ "$have_profiles" -eq 1 ] && [ -z "$profile" ]; then
        while IFS= read -r out; do
          call output-scale "$out" reset || failed=1
        done < <("$JQ" -r '.outputs[].name' <<<"$outputs")
      elif [ -n "$profile" ]; then
        # By connector name: the request takes one, so no id lookup
        # (and no `outputs` answer a hotplug could make stale first).
        while IFS=$'\t' read -r out scale; do
          call output-scale "$out" "$scale" || failed=1
        done < <("$JQ" -r '.outputs[] as $n | [$n, (.scale[$n] // "reset" | tostring)] | @tsv' <<<"$profile")
      fi

      # 3. Off: record, then call. Power takes ids, fresh from this
      # apply's `outputs` (ids are stable for the session, not across
      # unplug cycles).
      while IFS= read -r out; do
        [ -n "$out" ] || continue
        id=$("$JQ" -r --arg n "$out" '.outputs[] | select(.name == $n) | .id' <<<"$outputs")
        if [ -z "$id" ]; then
          echo "scoot-displays: '$out' left before its power could be set" >&2
          failed=1
          continue
        fi
        before="$keep"
        keep=$("$JQ" -c --arg n "$out" --argjson i "$id" \
          'if any(.[]; .name == $n and .id == $i) then . else . + [{name: $n, id: $i}] end' <<<"$keep")
        if ! write_held "$keep"; then
          # Never make an off this watcher could not undo later.
          keep="$before"
          failed=1
          continue
        fi
        if ! call output-power "$id" off; then
          failed=1
          keep="$before"
          write_held "$keep" || true
        fi
      done < <("$JQ" -r '.[]' <<<"$disabled")

      if [ "$have_profiles" -eq 0 ]; then
        if [ "$failed" -ne 0 ]; then
          echo "scoot-displays: no profiles configured, and releasing a held output failed" >&2
          return 1
        fi
        echo "scoot-displays: no profiles configured; changing nothing it did not change itself" >&2
        return 0
      fi
      if [ -z "$profile" ]; then
        if [ "$failed" -ne 0 ]; then
          echo "scoot-displays: no profile for $connected, and resetting it failed" >&2
          return 1
        fi
        echo "scoot-displays: no profile for $connected: scales back at the config file's, only its own power-offs undone" >&2
        return 0
      fi
      name=$("$JQ" -r '.name' <<<"$profile")
      if [ "$failed" -ne 0 ]; then
        echo "scoot-displays: profile '$name' NOT fully applied ($connected)" >&2
        return 1
      fi
      echo "scoot-displays: profile '$name' applied ($connected)" >&2
      return 0
    }

    # One apply at a time: a manual `apply` (or the idle policy's
    # resume) racing a hotplug-driven one would interleave their calls
    # and their held records, and both could report "applied" over a
    # set the other half-changed. `flock` on a file in the runtime dir:
    # the kernel drops the lock with the holder, so a killed applier
    # never wedges the next. Held around the apply only, never across
    # the watch loop's settle sleeps.
    do_apply_locked() {
      lock="$RUNTIME/scoot-displays.lock"
      {
        if ! "$FLOCK" -w 30 9; then
          echo "scoot-displays: another apply held $lock for 30 s; giving up" >&2
          return 1
        fi
        do_apply
      } 9>>"$lock"
    }

    do_watch() {
      # A clean end of stream (the session went away) is a failure for
      # a watcher: exit 1 so the unit's on-failure retry resubscribes
      # when the session is back, instead of idling unsubscribed.
      "$SCOOT" msg subscribe output | while IFS= read -r event; do
        case "$event" in
          # The start-up apply, once subscribed rather than before: a
          # plug between the two is then an event, never a miss.
          *'"type":"subscribed"'*)
            do_apply_locked || true
            continue
            ;;
          # Every add, removal, restore and in-place mode change: an
          # add fires for every plug, a first one or a replug of an
          # empty monitor included.
          *output_added* | *output_removed* | *output_restored* | *output_changed*) ;;
          *) continue ;;
        esac
        # Settle: hotplug arrives as a burst (added/restored, removed,
        # changed); wait out 2 s, then drain up to 1 s more, then apply
        # once.
        "$SLEEP" 2
        while IFS= read -r -t 1 _drained; do :; done
        do_apply_locked || true
      done
      return 1
    }

    case "''${1:-}" in
      status) do_status ;;
      apply) do_apply_locked ;;
      watch) do_watch ;;
      *)
        echo "usage: scoot-displays {status|apply|watch}" >&2
        exit 1
        ;;
    esac
  '';

in
{
  # Internal wiring, not a user option: the watcher, read by the idle
  # policy (`idle-home.nix` re-applies through it on resume, since its
  # `wlopm --on "*"` would otherwise relight a profile's `disabled`
  # output with nothing telling the watcher). Null unless the watcher
  # is installed (the slot on, on Linux), so the idle policy's resume
  # stays exactly `wlopm --on "*"` without it.
  options.programs.scoot.desktop.displays.watcher = lib.mkOption {
    type = lib.types.nullOr lib.types.package;
    default = null;
    internal = true;
    visible = false;
    description = ''
      Internal: the display-profile watcher (`scoot-displays`), read
      by the idle policy's resume. Not for direct use.
    '';
  };

  config = lib.mkMerge [
    (lib.mkIf dis.enable {
      assertions = [
        {
          # `mode` is not a profile field: a mode cannot apply live (a
          # reload refuses a changed mode -- the compositor never
          # modesets a running output), and the watcher owns no file to
          # stage one for the next login in. The option stays only so
          # a set `mode` is refused here with the way out, not as an
          # unknown option.
          assertion = lib.all (profile: profile.mode == { }) dis.profiles;
          message = ''
            programs.scoot.desktop.displays.profiles sets `mode`, which
            profiles do not take: the watcher applies scale and power
            live over IPC, and a mode cannot change live. Set a static
            mode per connector in programs.scoot.settings.outputs
            instead (e.g. `{ name = "DP-1"; mode = "3840x2160"; }`):
            it applies at session start, beside the watcher.
          '';
        }
        {
          # Profile names are unique (the name is what `status` prints
          # and the journal logs: two alike would be one too many).
          assertion =
            lib.length (lib.unique (map (profile: profile.name) dis.profiles)) == lib.length dis.profiles;
          message = ''
            programs.scoot.desktop.displays.profiles names two profiles
            alike: profile names are unique.
          '';
        }
        {
          # Connected sets are unique (first match wins, so a second
          # profile for one set would never apply).
          assertion =
            lib.length (lib.unique (map (profile: lib.sort lib.lessThan profile.outputs) dis.profiles))
            == lib.length dis.profiles;
          message = ''
            programs.scoot.desktop.displays.profiles matches two
            profiles on one connected set: each set appears once (first
            match wins).
          '';
        }
      ]
      ++ lib.concatMap (
        profile:
        let
          # Keys outside the profile's own set (a scale for an output
          # this profile never matches is a typo, refused here).
          outside = attrs: lib.subtractLists profile.outputs (lib.attrNames attrs);
        in
        [
          {
            assertion = profile.name != "" && builtins.match "^[[:space:]]*$" profile.name == null;
            message = ''
              programs.scoot.desktop.displays.profiles has a profile with
              an empty or blank name: name every profile (e.g.
              `name = "docked"`).
            '';
          }
          {
            assertion = profile.outputs != [ ];
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" matches nothing: `outputs` is empty (name
              the connected set, e.g. `outputs = [ "eDP-1" "DP-1" ]`).
            '';
          }
          {
            assertion = lib.all (
              name: name != "" && builtins.match "^[[:space:]]*$" name == null
            ) profile.outputs;
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" names an empty or blank output: connector
              names come from `scoot msg outputs` (e.g. `DP-1`).
            '';
          }
          {
            assertion = outside profile.scale == [ ];
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" scales an output outside its `outputs`:
              ${lib.concatStringsSep ", " (outside profile.scale)}.
            '';
          }
          {
            assertion = lib.subtractLists profile.outputs profile.disabled == [ ];
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" disables an output outside its `outputs`:
              ${lib.concatStringsSep ", " (lib.subtractLists profile.outputs profile.disabled)}.
            '';
          }
          {
            # A profile that disables its whole set leaves every screen
            # dark the moment it matches -- at every session start, if
            # that is the set the session starts with. Refused here
            # rather than applied (guarded on a non-empty set, which
            # its own refusal above covers).
            assertion = profile.outputs == [ ] || lib.subtractLists profile.disabled profile.outputs != [ ];
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" disables every output it matches
              (${lib.concatStringsSep ", " profile.outputs}), which would
              leave no screen on: keep at least one output of `outputs`
              out of `disabled`.
            '';
          }
          {
            # The compositor's own range (`output_scale.rs`): outside
            # it `output-scale` refuses, so fail here instead of
            # shipping a profile that fails every apply.
            assertion = lib.all (scale: scale >= 0.5 && scale <= 4.0) (lib.attrValues profile.scale);
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" scales outside 0.5 to 4.0 (the
              compositor's own range).
            '';
          }
        ]
      ) dis.profiles;

      # The watcher (Linux only: the session it serves is
      # Linux-only, the way the OSD scripts are).
      home.packages = lib.optionals isLinux [ scootDisplays ];
      programs.scoot.desktop.displays.watcher = lib.mkIf isLinux scootDisplays;
    })

    # The profiles file and the unit (Linux only, beside the watcher
    # above -- kept in its own element so a non-Linux evaluation
    # carries neither file nor unit, the way the OSD config does).
    (lib.mkIf (dis.enable && isLinux) {
      # The profiles the watcher matches, beside the unit that runs it.
      # Pure data, so it renders wherever the slot is on (without the
      # unit beside it the file simply waits, the way a chooser config
      # without its portal does).
      xdg.configFile."scoot/displays.json".source = profilesFile;

      # The watcher itself: `watch` from session start, re-matching on
      # every output event. Wanted by `scoot-session.target` -- scoot's
      # own session scope (started by the launcher past the display
      # import, so the display is there when the watcher starts) --
      # never the shared `graphical-session.target`, which every other
      # desktop reaches too and would start this watcher inside someone
      # else's session. Retried like the bar's unit rather than
      # conditioned (a skipped start is never retried). No restart
      # trigger on the config file: the watcher never reads it, and a
      # rebuild that replaces it does not reload the session either, so
      # the scales the watcher applied stay applied.
      systemd.user.services.scoot-displays = {
        Unit = {
          Description = "scoot display profiles (scale and power follow the connected set)";
          PartOf = [ "scoot-session.target" ];
          After = [ "scoot-session.target" ];
          # A new watcher or new profiles restarts the daemon (it
          # re-applies at start, so the running session converges at
          # once instead of at the next hotplug).
          X-Restart-Triggers = [
            "${scootDisplays}"
            "${profilesFile}"
          ];
          # Unending retries (`StartLimitIntervalSec` lives in `[Unit]`:
          # systemd ignores it in `[Service]`).
          StartLimitIntervalSec = 0;
        };
        Service = {
          ExecStart = "${scootDisplays}/bin/scoot-displays watch";
          Restart = "on-failure";
          RestartSec = 2;
        };
        Install.WantedBy = [ "scoot-session.target" ];
      };
    })

    # The profile turns the slot on (still individually disable-able
    # at plain priority, the way the clipboard slot works). With no
    # profiles the watcher sets no scale and turns nothing off (it only
    # turns back on an output it turned off itself under an earlier
    # list), so turning it on is safe before any profile exists.
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.displays.enable = lib.mkDefault true;
    })
  ];
}
