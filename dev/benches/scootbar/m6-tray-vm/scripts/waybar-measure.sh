#!/bin/bash
# Waybar (nixpkgs 0.15.0) with only a tray module, same harness as measure.sh, for rule 2's comparison.
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
WB=/nix/store/asv7n18sz079pgq39shslji3qw5yfs4l-waybar-0.15.0/bin/waybar
ITEMS=${1:-1}
R=/tmp/tray-takeover-run/m-waybar-$ITEMS; rm -rf $R; mkdir -p $R; chmod 700 $R
S=/tmp/tray-takeover-run/s-waybar-$ITEMS; rm -rf $S; mkdir -p $S
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 1600 --height 1000 --socket $R/scoot.sock --config /dev/null >$S/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
dbus-daemon --session --nofork --address=unix:path=$S/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $S/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$S/bus
printf '{"layer":"top","position":"top","height":28,"modules-left":["tray"]}\n' > $S/waybar.json
printf '* { font-size: 13px; }\n' > $S/style.css
WAYLAND_DISPLAY=$WD $WB -c $S/waybar.json -s $S/style.css >$S/waybar.log 2>&1 &
BAR=$!
sleep 3

ITEMPIDS=""
for n in $(seq 1 $ITEMS); do
  $PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-$$-$n $S/item$n.log $S/flag$n $((n*15)) >$S/item$n.out 2>&1 &
  ITEMPIDS="$ITEMPIDS $!"
done
sleep 12
snap() {
  local p=$BAR
  echo "$1 pid=$p rss_kb=$(awk '/VmRSS/{print $2}' /proc/$p/status) pss_kb=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup) threads=$(awk '/Threads/{print $2}' /proc/$p/status) fds=$(ls /proc/$p/fd | wc -l) vol=$(awk '/^voluntary_ctxt/{print $2}' /proc/$p/status) nonvol=$(awk '/nonvoluntary_ctxt/{print $2}' /proc/$p/status) utime_ticks=$(awk '{print $14}' /proc/$p/stat) stime_ticks=$(awk '{print $15}' /proc/$p/stat)"
}
snap t0
# thread-summed context switches too (waybar is multi-threaded)
tsum() { cat /proc/$BAR/task/*/status | awk '/^voluntary_ctxt/{v+=$2} /^nonvoluntary_ctxt/{n+=$2} END{print "all_threads_vol="v" nonvol="n}'; }
tsum
sleep 60
snap t60
tsum
echo "items seen by item scripts: $(grep -c getall $S/item1.log 2>/dev/null)"
echo "waybar.log:"; head -5 $S/waybar.log
kill $ITEMPIDS $BAR $DBD $SCOOT 2>/dev/null
