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
# three backends for a packaging slot -- and a reload already refuses
# runtime mode changes -- while `subscribe` (events), `outputs`
# (matching), `reload` (scale, live) and `output-power` give matching
# plus applying with no compositor change at all. So the watcher below
# speaks scoot's own IPC, and wakes on hotplug the same way kanshi
# does (blocking read, no polling).
#
# The option shapes live in `./desktop.nix` (shared with the NixOS
# side); there are no `package` defaults here because the slot installs
# no daemon of its own (the watcher is built below from `jq`, `gawk`
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
  awkBin = lib.getExe' pkgs.gawk "awk";
  grepBin = lib.getExe pkgs.gnugrep;
  cmpBin = lib.getExe' pkgs.diffutils "cmp";
  mkdirBin = lib.getExe' pkgs.coreutils "mkdir";
  dirnameBin = lib.getExe' pkgs.coreutils "dirname";
  mktempBin = lib.getExe' pkgs.coreutils "mktemp";
  sleepBin = lib.getExe' pkgs.coreutils "sleep";
  catBin = lib.getExe' pkgs.coreutils "cat";
  mvBin = lib.getExe' pkgs.coreutils "mv";
  rmBin = lib.getExe' pkgs.coreutils "rm";
  tailBin = lib.getExe' pkgs.coreutils "tail";

  # The profiles, as the watcher matches them: the `outputs` set per
  # profile (exact connected set, connector names), the per-output
  # scale/mode tables, and the power-off list. Rendered once here, read
  # by absolute store path (a changed file restarts the unit, so the
  # watcher never runs stale profiles).
  profilesFile = pkgs.writeText "scoot-displays.json" (builtins.toJSON dis.profiles);

  # The watcher's managed block in the live config file: everything
  # between the markers below is the last applied profile, rewritten
  # (or removed, when nothing matches) by `apply`. The block is the
  # only thing the watcher owns in that file -- user settings around
  # it are never touched -- which is also why `settings.outputs` is
  # refused beside `enable` (two writers for `[[outputs]]` would fight,
  # the first entry silently winning).
  beginMark = "# BEGIN scoot-displays (managed by scoot-displays apply; do not edit)";
  endMark = "# END scoot-displays";

  # The watcher: `status` prints the matched profile, `apply` matches
  # once and applies, `watch` applies at start and re-applies on every
  # output event. Matching is by exact connected connector-name set
  # (first profile in list order wins); make and model cannot key a
  # profile (scoot's IPC does not report them -- `outputs` carries
  # names, rects, scales and power only), the same key `[[outputs]]`
  # already matches on.
  scootDisplays = pkgs.writeShellScriptBin "scoot-displays" ''
    set -u
    PROFILES=${profilesFile}
    CONFIG_REL=${lib.escapeShellArg cfg.configFile}
    SCOOT=${lib.escapeShellArg scootBin}
    JQ=${lib.escapeShellArg jqBin}
    AWK=${lib.escapeShellArg awkBin}
    GREP=${lib.escapeShellArg grepBin}
    CMP=${lib.escapeShellArg cmpBin}
    MKDIR=${lib.escapeShellArg mkdirBin}
    DIRNAME=${lib.escapeShellArg dirnameBin}
    MKTEMP=${lib.escapeShellArg mktempBin}
    SLEEP=${lib.escapeShellArg sleepBin}
    CAT=${lib.escapeShellArg catBin}
    MV=${lib.escapeShellArg mvBin}
    RM=${lib.escapeShellArg rmBin}
    TAIL=${lib.escapeShellArg tailBin}
    BEGIN_MARK=${lib.escapeShellArg beginMark}
    END_MARK=${lib.escapeShellArg endMark}

    config_file() {
      base="''${XDG_CONFIG_HOME:-$HOME/.config}"
      printf '%s/%s' "$base" "$CONFIG_REL"
    }

    # The live outputs, or a loud refusal (no session: exit 1, so the
    # unit's on-failure retry covers a watcher started too early).
    live_outputs() {
      "$SCOOT" msg outputs 2>/dev/null || {
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

    # The managed `[[outputs]]` block for a matched profile, on stdout:
    # one entry per output in the profile's set that sets anything (a
    # scale, a mode, or both). Scales render as TOML floats (`2` would
    # be an integer, which the compositor's float field refuses), modes
    # verbatim (eval already pinned them to `WxH`).
    render_block() {
      profile=$1
      "$JQ" -r '.outputs[]' <<<"$profile" | while IFS= read -r name; do
        # Connector names come from the kernel (`DP-1`, `eDP-1`,
        # `headless-2`): anything outside that alphabet never reaches
        # the TOML (a stray quote would refuse the whole config file
        # at load, failing safe but loud -- skip the entry instead).
        case "$name" in
          "" | *[!A-Za-z0-9_.+-]*)
            echo "scoot-displays: refusing odd connector name '$name'" >&2
            continue
            ;;
        esac
        scale=$("$JQ" -r --arg n "$name" '.scale[$n] // empty' <<<"$profile")
        mode=$("$JQ" -r --arg n "$name" '.mode[$n] // empty' <<<"$profile")
        if [ -z "$scale" ] && [ -z "$mode" ]; then
          continue
        fi
        printf '[[outputs]]\nname = "%s"\n' "$name"
        if [ -n "$scale" ]; then
          case "$scale" in
            *.*) ;;
            *) scale="$scale.0" ;;
          esac
          printf 'scale = %s\n' "$scale"
        fi
        if [ -n "$mode" ]; then
          printf 'mode = "%s"\n' "$mode"
        fi
      done
    }

    # The managed block a config file holds now (empty when it holds
    # none): the lines between the markers.
    managed_now() {
      "$AWK" -v begin="$BEGIN_MARK" -v end="$END_MARK" '
        $0 == begin { inside = 1; next }
        $0 == end { inside = 0; next }
        inside { print }
      ' "$1"
    }

    # Replaces the managed block in the live config with the text on
    # stdin (empty stdin removes the block, leaving no trace). Appends
    # the block when the file has no markers yet, creates the file (and
    # its directory) when missing. Prints whether the file changed
    # (`changed` / `same`), so the caller reloads only on a real
    # change; the previous block is remembered beside the config, never
    # in it.
    place_block() {
      cfg=$(config_file)
      dir=$("$DIRNAME" "$cfg")
      [ -d "$dir" ] || "$MKDIR" -p "$dir"
      new=$("$MKTEMP")
      "$CAT" >"$new"
      # An empty block with nowhere to clear is a no-op (a fresh
      # config stays marker-free instead of collecting empty managed
      # regions on every unmatched apply).
      if [ ! -s "$new" ] && [ ! -f "$cfg.prev-displays" ]; then
        if [ ! -f "$cfg" ] || ! "$GREP" -qF "$BEGIN_MARK" "$cfg"; then
          "$RM" -f "$new"
          echo same
          return 0
        fi
      fi
      if [ -f "$cfg" ] && "$GREP" -qF "$BEGIN_MARK" "$cfg"; then
        if [ -s "$new" ]; then
          "$AWK" -v begin="$BEGIN_MARK" -v end="$END_MARK" -v new="$new" '
            $0 == begin { print; while ((getline line < new) > 0) print line; skip = 1; next }
            $0 == end { skip = 0 }
            !skip { print }
          ' "$cfg" >"$cfg.tmp" && "$MV" "$cfg.tmp" "$cfg"
        else
          "$AWK" -v begin="$BEGIN_MARK" -v end="$END_MARK" '
            $0 == begin { skip = 1; next }
            $0 == end { skip = 0; next }
            !skip { print }
          ' "$cfg" >"$cfg.tmp" && "$MV" "$cfg.tmp" "$cfg"
        fi
      else
        if [ -f "$cfg" ] && [ -n "$("$TAIL" -c 1 "$cfg")" ]; then
          printf '\n' >>"$cfg"
        fi
        {
          printf '%s\n' "$BEGIN_MARK"
          "$CAT" "$new"
          # The block on stdin may lack its trailing newline (command
          # substitution strips it): terminate it, or the end marker
          # glues onto its last line.
          if [ -n "$("$TAIL" -c 1 "$new")" ]; then
            printf '\n'
          fi
          printf '%s\n' "$END_MARK"
        } >>"$cfg"
      fi
      "$RM" -f "$new"
      if [ -f "$cfg.prev-displays" ] && "$CMP" -s "$cfg.prev-displays" <(managed_now "$cfg"); then
        echo same
      else
        managed_now "$cfg" >"$cfg.prev-displays"
        echo changed
      fi
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

    do_apply() {
      outputs=$(live_outputs) || return 1
      connected=$(connected_names "$outputs")
      profile=$(match_profile "$connected")
      if [ -z "$profile" ]; then
        echo "scoot-displays: no profile for $connected: clearing the managed block, leaving power alone" >&2
        if [ "$(printf '%s' "" | place_block)" = changed ]; then
          "$SCOOT" msg reload >/dev/null || echo "scoot-displays: reload failed" >&2
        fi
        return 0
      fi
      name=$("$JQ" -r '.name' <<<"$profile")
      block=$(render_block "$profile")
      if [ "$(printf '%s' "$block" | place_block)" = changed ]; then
        echo "scoot-displays: profile '"$name"' applied ($connected)" >&2
        "$SCOOT" msg reload >/dev/null || echo "scoot-displays: reload failed" >&2
        if "$JQ" -e '.mode | length > 0' <<<"$profile" >/dev/null; then
          echo "scoot-displays: profile '"$name"' sets modes, which take effect at the next login (a reload never modesets a running output)" >&2
        fi
      fi
      # Power: every connected output of the set is on unless the
      # profile disables it (so a stale off from another profile never
      # lingers). Names resolve to the live ids fresh each apply: ids
      # are stable for the session, not across unplug cycles.
      "$JQ" -r '.outputs[]' <<<"$profile" | while IFS= read -r out; do
        id=$("$JQ" -r --arg n "$out" '.outputs[] | select(.name == $n) | .id // empty' <<<"$outputs")
        if [ -z "$id" ]; then
          echo "scoot-displays: profile '"$name"' names '"$out"', which is not connected; skipping its power" >&2
          continue
        fi
        if "$JQ" -e --arg n "$out" '.disabled | index($n) != null' <<<"$profile" >/dev/null; then
          "$SCOOT" msg output-power "$id" off >/dev/null || echo "scoot-displays: powering off '"$out"' failed" >&2
        else
          "$SCOOT" msg output-power "$id" on >/dev/null || echo "scoot-displays: powering on '"$out"' failed" >&2
        fi
      done
      return 0
    }

    do_watch() {
      do_apply || true
      # A clean end of stream (the session went away) is a failure for
      # a watcher: exit 1 so the unit's on-failure retry resubscribes
      # when the session is back, instead of idling unsubscribed.
      "$SCOOT" msg subscribe output | while IFS= read -r event; do
        case "$event" in
          *output_removed* | *output_restored* | *output_changed*) ;;
          *) continue ;;
        esac
        # Settle: hotplug arrives as a burst (remove/restore/changed);
        # wait out 2 s, then drain up to 1 s more, then apply once.
        "$SLEEP" 2
        while IFS= read -r -t 1 _drained; do :; done
        do_apply || true
      done
      return 1
    }

    case "''${1:-}" in
      status) do_status ;;
      apply) do_apply ;;
      watch) do_watch ;;
      *)
        echo "usage: scoot-displays {status|apply|watch}" >&2
        exit 1
        ;;
    esac
  '';

in
{
  config = lib.mkMerge [
    (lib.mkIf dis.enable {
      assertions = [
        {
          # The watcher owns `[[outputs]]` in the live file; a static
          # entry beside it would fight the managed block (the first
          # entry wins, silently), so it fails here instead.
          assertion = !(cfg.settings ? outputs);
          message = ''
            programs.scoot.desktop.displays.enable is set but
            programs.scoot.settings has `outputs`: the displays watcher
            owns `[[outputs]]` (its managed block), so a static entry
            would shadow it. Move per-output scales into
            programs.scoot.desktop.displays.profiles[].scale instead.
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
            assertion = outside profile.mode == [ ];
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" modes an output outside its `outputs`:
              ${lib.concatStringsSep ", " (outside profile.mode)}.
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
            # The compositor's own range (`output_scale.rs`): outside
            # it the loader clamps with a warning, so fail here
            # instead of shipping a scale that never applies as
            # written.
            assertion = lib.all (scale: scale >= 0.5 && scale <= 4.0) (lib.attrValues profile.scale);
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" scales outside 0.5 to 4.0 (the
              compositor's own range).
            '';
          }
          {
            assertion = lib.all (mode: builtins.match "^[1-9][0-9]*x[1-9][0-9]*$" mode != null) (
              lib.attrValues profile.mode
            );
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" modes outside `WxH` (e.g.
              `mode = { "DP-1" = "3840x2160"; }`).
            '';
          }
          {
            # At most `--width`/`--height` (`cli.rs`): larger asks no
            # connector answered at startup either. Skipped when the
            # shape already failed (a null match is not a list to
            # map over -- the shape refusal above names it, so this
            # stays quiet and each bad mode fails exactly once).
            assertion = lib.all (
              mode:
              let
                parts = builtins.match "^([1-9][0-9]*)x([1-9][0-9]*)$" mode;
              in
              parts == null || lib.all (dim: dim <= 65535) (map builtins.fromJSON parts)
            ) (lib.attrValues profile.mode);
            message = ''
              programs.scoot.desktop.displays.profiles profile
              "${profile.name}" modes past 65535 pixels (the largest a
              headless or nested output may ask for).
            '';
          }
        ]
      ) dis.profiles;

      # The watcher (Linux only: the session it serves is
      # Linux-only, the way the OSD scripts are).
      home.packages = lib.optionals isLinux [ scootDisplays ];
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
      # trigger on the config file: the watcher re-reads the live file
      # every apply, and the profiles file below restarts it.
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
    # profiles the watcher stays idle (no match, no block, no power
    # changes), so turning it on is safe before any profile exists.
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.displays.enable = lib.mkDefault true;
    })
  ];
}
