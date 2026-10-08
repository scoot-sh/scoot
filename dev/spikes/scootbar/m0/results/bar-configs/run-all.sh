#!/bin/bash
# Every bar on scoot (wayland-1) then sway (wayland-2), one at a time.
B=/tmp/sb-bench; . $B/env.sh
export SCOOT_SOCKET=/tmp/sb-bench/rt/scoot-ipc.sock SWAYSOCK=$(ls /tmp/sb-bench/rt/sway-ipc.*.sock)
C=/tmp/sb-m0-target/debug/scootctl; SM=/nix/store/5ddkfdxnq991rfzn2f6n5w1kd6dvaqp3-sway-1.12/bin/swaymsg
H=/tmp/sb-m0/dev/spikes/scootbar/m0/bench/bar-bench.py
declare -A BIN=([yambar]=/nix/store/csn1vphr40wshc60v5a5mdjc5yjr4j8n-yambar-1.11.0/bin/yambar [waybar]=/nix/store/wlngldlifb2jlab2gmm5gbsb9r8wrch6-waybar-0.15.0/bin/waybar [ironbar]=/nix/store/8iqc8rcid97d63sih16b239sxr1y51qv-ironbar-0.19.0/bin/ironbar [ashell]=/nix/store/fz9as7cihw4xbb3zmbfmgmfbz6hklqzh-ashell-0.10.0/bin/ashell)
for comp in scoot sway; do
  for b in yambar waybar ironbar ashell; do
    cfg=$B/cfg-$b
    if [ $comp = scoot ]; then
      export WAYLAND_DISPLAY=wayland-1
      sw0="$C action focus-workspace-index 0"; sw1="$C action focus-workspace-index 1"
    else
      export WAYLAND_DISPLAY=wayland-2
      sw0="$SM workspace number 1"; sw1="$SM workspace number 2"
      [ $b = yambar ] && cfg=$B/cfg-yambar-sway
    fi
    echo "=== $comp $b $(date -u +%FT%TZ)"
    XDG_CONFIG_HOME=$cfg python3 $H --runs 5 --settle 30 --window 300 --switch-cmd "$sw1" "$sw0" --switch-secs 60 --switch-hz 4 -- ${BIN[$b]}
    $sw0 >/dev/null 2>&1
    sleep 2
  done
done
echo "=== done $(date -u +%FT%TZ)"
