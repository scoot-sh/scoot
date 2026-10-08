#!/bin/bash
# usage: measure.sh LABEL BIN CONFIG MODE
# MODE: nobus | bus | bluez | flood
# One scoot --headless, one private dbus-daemon as the *system* bus (none
# for `nobus`), one scootbar, and the BlueZ the mode says: `bluez` one
# idle perl peer (adapter on, one device connected, then silent), `flood`
# the same peer blasting 10,000 PropertiesChanged. Samples go outside the
# runtime dir (the bar's inotify watch is on it while there is no bus):
# S, not R.
LABEL=$1; BIN=$2; CONFIG=$3; MODE=${4:-bus}
HERE=$(dirname "$0")
FONT=/nix/store/33wfvwgr4vy0gaznf3wafla7mfni1znw-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf
ROOT=/tmp/bt-bench-run
R=$ROOT/m-$LABEL; rm -rf $R; mkdir -p $R; chmod 700 $R
S=$ROOT/s-$LABEL; rm -rf $S; mkdir -p $S
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 1600 --height 1000 --socket $R/scoot.sock --config /dev/null >$S/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
DBD=
PEER=
if [ "$MODE" != nobus ]; then
  printf '%s' '<busconfig><type>session</type><auth>EXTERNAL</auth><listen>unix:path='$S'/bus</listen><policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>' > $S/bus.conf
  dbus-daemon --nofork --config-file $S/bus.conf >/dev/null 2>&1 &
  DBD=$!
  for i in $(seq 1 100); do [ -S $S/bus ] && break; sleep 0.1; done
  export DBUS_SYSTEM_BUS_ADDRESS=unix:path=$S/bus
else
  export DBUS_SYSTEM_BUS_ADDRESS=unix:path=$R/no-bus
fi
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $CONFIG >$S/bar.log 2>&1 &
BAR=$!
sleep 2
case $MODE in
  bluez) perl $HERE/bluez.pl $S/bus idle >$S/peer.log 2>&1 & PEER=$! ;;
  flood) perl $HERE/bluez.pl $S/bus flood 10000 >$S/peer.log 2>&1 & PEER=$! ;;
esac
sleep 12
snap() {
  local p=$BAR
  echo "$1 rss_kb=$(awk '/VmRSS/{print $2}' /proc/$p/status) hwm_kb=$(awk '/VmHWM/{print $2}' /proc/$p/status) pss_kb=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup) threads=$(awk '/Threads/{print $2}' /proc/$p/status) fds=$(ls /proc/$p/fd | wc -l) vol=$(awk '/^voluntary_ctxt/{print $2}' /proc/$p/status) nonvol=$(awk '/nonvoluntary_ctxt/{print $2}' /proc/$p/status) utime_ticks=$(awk '{print $14}' /proc/$p/stat) stime_ticks=$(awk '{print $15}' /proc/$p/stat)"
}
snap t0 | tee $S/t0.txt
sleep 60
snap t60 | tee $S/t60.txt
echo "query:"; WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | head -c 600 | tee $S/query.txt; echo
echo "binary_bytes=$(stat -c %s $BIN)"
kill $BAR $PEER $DBD $SCOOT 2>/dev/null
sleep 0.5
echo "bar.log lines: $(wc -l < $S/bar.log)"; head -3 $S/bar.log
