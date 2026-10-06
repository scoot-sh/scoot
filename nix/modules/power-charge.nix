# The desk-aware charge-limit script (`scoot-charge`), ported from the
# maintainer's hand-wired `charge.nix` on the Asahi M2 (which this
# obsoletes when adopted there): the battery normally stops at `limit`
# percent, charges to 100% once on demand (until the next unplug), and
# refills to 100% on its own after `fullAfter` seconds on battery (a
# trip needs the range), dropping back after `tripEndsAfter` seconds
# straight on the charger. Root re-syncs on every AC change (udev) and
# every 5 minutes (a timer); the bar button needs no password.
#
# Takes the option values as arguments (so `nix/tests.nix` behavior
# tests run the real script against a fake sysfs through the
# `SCOOT_CHARGE_*` overrides below, the way the notification feed and
# the clipboard picker are tested). `battery` null auto-detects the
# first supply with a `charge_control_end_threshold` node, so one
# config travels across machines (`macsmc-battery` on Apple silicon,
# `BAT0` on most laptops).
{
  pkgs,
  lib,
  limit,
  fullAfter,
  tripEndsAfter,
  battery,
}:

pkgs.writeShellScriptBin "scoot-charge" ''
  set -u
  LIMIT=${toString limit} TRIP_AFTER=${toString fullAfter} TRIP_ENDS=${toString tripEndsAfter}
  PATH=${
    lib.makeBinPath [
      pkgs.coreutils
      pkgs.systemd
      pkgs.util-linux
      pkgs.gawk
    ]
  }:$PATH
  now=$(date +%s)

  # The battery directory: explicit (`SCOOT_CHARGE_BATTERY`, else the
  # `battery` option as a `/sys/class/power_supply` name), else the
  # first supply with a `charge_control_end_threshold` node. Empty
  # where the hardware has none -- every entry below degrades on that
  # (the service is inert there by design, never refused: eval cannot
  # see the machine).
  BATTERY_OPT=${lib.escapeShellArg (if battery == null then "" else battery)}
  if [ -n "''${SCOOT_CHARGE_BATTERY:-}" ]; then
    B="$SCOOT_CHARGE_BATTERY"
  elif [ -n "$BATTERY_OPT" ]; then
    B="/sys/class/power_supply/$BATTERY_OPT"
  else
    B=""
    for d in /sys/class/power_supply/*; do
      if [ -f "$d/charge_control_end_threshold" ]; then B="$d"; break; fi
    done
  fi

  # The AC online file: explicit (`SCOOT_CHARGE_AC`), else the first
  # readable `online` beside a Mains supply (`macsmc-ac` on Apple
  # silicon, `AC`/`ACAD`/`ADP1` elsewhere). Missing means unknown, and
  # unknown counts as plugged (never accumulate phantom time on
  # battery toward a trip from a missing file).
  if [ -n "''${SCOOT_CHARGE_AC:-}" ]; then
    AC="$SCOOT_CHARGE_AC"
  else
    AC=""
    for d in /sys/class/power_supply/macsmc-ac /sys/class/power_supply/AC /sys/class/power_supply/ACAD /sys/class/power_supply/ADP1 /sys/class/power_supply/AC0; do
      if [ -f "$d/online" ]; then AC="$d/online"; break; fi
    done
    if [ -z "$AC" ]; then
      for d in /sys/class/power_supply/*; do
        if [ -f "$d/online" ] && [ "$(cat "$d/type" 2>/dev/null)" = "Mains" ]; then AC="$d/online"; break; fi
      done
    fi
  fi

  S="''${SCOOT_CHARGE_STATE:-/var/lib/scoot-charge}"
  mode() { cat "$S/mode" 2>/dev/null || echo limit; }
  setmode() { echo "$1" > "$S/mode"; }
  get() { cat "$S/$1" 2>/dev/null || echo "$2"; }
  ac_online() { if [ -n "$AC" ]; then cat "$AC"; else echo 1; fi; }

  # The bar button (the push module's `charge` cell): the state as
  # text, which is what reads in the bar's default font (no battery
  # glyph in it reads at bar size -- the envelope lesson from the
  # notification icons), with the explanation in the tooltip. Root
  # fans out to every logged-in user's every bar display; a user run
  # talks to its own bar. Missing bars are fine (`|| true`): push is
  # best-effort, sync is what holds the threshold.
  push() {
    case $(mode) in
      once)  text=full tip="Charging to 100% until you unplug. Click for 80%." ;;
      trip)  text=trip tip="Refilling to 100% after time on battery; back to 80% after a day on the charger. Click for 80%." ;;
      *)     text=80 tip="Charging stops at 80% to save the battery. Click to charge to full once." ;;
    esac
    json=$(printf '{"text":"%s","tooltip":"%s"}' "$text" "$tip")
    if [ "$(id -u)" = 0 ]; then
      for uid in $(loginctl list-users --no-legend 2>/dev/null | awk '{print $1}'); do
        user=$(id -nu "$uid" 2>/dev/null) || continue
        bar=/etc/profiles/per-user/$user/bin/scootbar
        [ -x "$bar" ] && [ -d "/run/user/$uid" ] || continue
        # one scootbar per display: its socket names the display to talk to
        for sock in /run/user/$uid/scootbar-*.sock; do
          [ -S "$sock" ] || continue
          wd=''${sock##*/scootbar-}; wd=''${wd%.sock}
          runuser -u "$user" -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY="$wd" "$bar" msg set charge "$json" >/dev/null 2>&1 || true
        done
      done
    else
      scootbar msg set charge "$json" >/dev/null 2>&1 || true
    fi
  }

  # No charge-control node: inert (one line, exit 0). Every state
  # transition still runs, so a later kernel (or a docked battery)
  # picks up coherent state instead of a stale mode.
  have_node() { [ -n "$B" ] && [ -f "$B/charge_control_end_threshold" ]; }

  apply() {
    have_node || return 0
    case $(mode) in once|trip) want=100 ;; *) want=$LIMIT ;; esac
    [ "$(cat "$B/charge_control_end_threshold")" = "$want" ] || echo "$want" > "$B/charge_control_end_threshold"
  }

  # Follow AC transitions: called on every udev event and timer tick, so a
  # missed event is caught within five minutes.
  sync() {
    online=$(ac_online); last=$(get last_ac "$online")
    if [ "$online" != "$last" ]; then
      if [ "$online" = 0 ]; then
        echo "$now" > "$S/unplugged_at"
        [ "$(mode)" = once ] && setmode limit   # once lasts until the next unplug
      else
        echo "$now" > "$S/plugged_at"
        away=$(( now - $(get unplugged_at "$now") ))
        [ "$(mode)" = limit ] && [ "$TRIP_AFTER" -gt 0 ] && [ "$away" -ge "$TRIP_AFTER" ] && setmode trip
      fi
    fi
    echo "$online" > "$S/last_ac"   # the baseline for the next sync, every time
    if [ "$(mode)" = trip ] && [ "$online" = 1 ] && [ "$TRIP_ENDS" -gt 0 ] && [ $(( now - $(get plugged_at "$now") )) -ge "$TRIP_ENDS" ]; then
      setmode limit
    fi
  }

  case ''${1:-status} in
    toggle)    if [ "$(mode)" = limit ]; then setmode once; else setmode limit; fi; apply; push ;;
    full-once) setmode once; apply; push ;;
    limit)     setmode limit; apply; push ;;
    sync)      sync; apply; push ;;
    push)      push ;;
    status)
      if have_node; then threshold=$(cat "$B/charge_control_end_threshold"); else threshold=unsupported; fi
      echo "mode=$(mode) threshold=$threshold ac=$(ac_online) capacity=$(cat "$B/capacity" 2>/dev/null || echo unknown)%"
      ;;
    *) echo "usage: scoot-charge [status|toggle|full-once|limit|sync|push]" >&2; exit 2 ;;
  esac
''
