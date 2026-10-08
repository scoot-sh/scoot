#!/bin/bash
# usage: measure.sh LABEL BIN CONFIG ITEMS(int) [bus=yes|no]
# One scoot --headless, one private dbus-daemon, one scootbar, ITEMS jeepney
# items. Samples are written OUTSIDE the runtime dir (the bar's inotify watch
# is on it while there is no bus): S, not R.
LABEL=$1; BIN=$2; CONFIG=$3; ITEMS=${4:-0}; BUS=${5:-yes}
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/tray-takeover-run/m-$LABEL; rm -rf $R; mkdir -p $R; chmod 700 $R
S=/tmp/tray-takeover-run/s-$LABEL; rm -rf $S; mkdir -p $S
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 1600 --height 1000 --socket $R/scoot.sock --config /dev/null >$S/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
if [ "$BUS" = yes ]; then
  dbus-daemon --session --nofork --address=unix:path=$S/bus >/dev/null 2>&1 &
  DBD=$!
  for i in $(seq 1 100); do [ -S $S/bus ] && break; sleep 0.1; done
  export DBUS_SESSION_BUS_ADDRESS=unix:path=$S/bus
else
  export DBUS_SESSION_BUS_ADDRESS=unix:path=$R/bus
fi
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $CONFIG >$S/bar.log 2>&1 &
BAR=$!
sleep 2
ITEMPIDS=""
for n in $(seq 1 $ITEMS); do
  $PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-$$-$n $S/item$n.log $S/flag$n $((n*15)) >$S/item$n.out 2>&1 &
  ITEMPIDS="$ITEMPIDS $!"
done
sleep 12
snap() {
  local p=$BAR
  echo "$1 rss_kb=$(awk '/VmRSS/{print $2}' /proc/$p/status) hwm_kb=$(awk '/VmHWM/{print $2}' /proc/$p/status) pss_kb=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup) threads=$(awk '/Threads/{print $2}' /proc/$p/status) fds=$(ls /proc/$p/fd | wc -l) vol=$(awk '/^voluntary_ctxt/{print $2}' /proc/$p/status) nonvol=$(awk '/nonvoluntary_ctxt/{print $2}' /proc/$p/status) utime_ticks=$(awk '{print $14}' /proc/$p/stat) stime_ticks=$(awk '{print $15}' /proc/$p/stat)"
}
snap t0 | tee $S/t0.txt
sleep 60
snap t60 | tee $S/t60.txt
echo "query:"; WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | head -c 400 | tee $S/query.txt; echo
echo "binary_bytes=$(stat -c %s $BIN)"
kill $BAR $ITEMPIDS $DBD $SCOOT 2>/dev/null
sleep 0.5
echo "bar.log lines: $(wc -l < $S/bar.log)"; head -3 $S/bar.log
