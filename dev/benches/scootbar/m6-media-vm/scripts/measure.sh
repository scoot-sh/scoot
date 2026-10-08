#!/bin/bash
# usage: measure.sh LABEL BIN CONFIG MODE
# MODE: nobus | bus | paused | playing | eight | mpv | mpvlavfi
# One scoot --headless, one private dbus-daemon (none for `nobus`), one
# scootbar, and the players the mode says: `paused`/`playing` one jeepney
# stub (player.py), `eight` eight playing stubs, `mpv` one real mpv with its
# MPRIS script. Samples go outside the runtime dir (the bar's inotify watch
# is on it while there is no bus): S, not R.
LABEL=$1; BIN=$2; CONFIG=$3; MODE=${4:-bus}
HERE=$(dirname "$0")
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
MPV=/nix/store/95shw2n7gm3d68zi6vkg19cdk5y5slrq-mpv-with-scripts-0.41.0/bin/mpv
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
ROOT=/tmp/media-bench-run
R=$ROOT/m-$LABEL; rm -rf $R; mkdir -p $R; chmod 700 $R
S=$ROOT/s-$LABEL; rm -rf $S; mkdir -p $S
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 1600 --height 1000 --socket $R/scoot.sock --config /dev/null >$S/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
DBD=
if [ "$MODE" != nobus ]; then
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
PLAYERS=""
case $MODE in
  paused)  $PY $HERE/player.py org.mpris.MediaPlayer2.stub1 $S/p1.log $S/f1 Paused >$S/p1.out 2>&1 & PLAYERS="$!" ;;
  playing) $PY $HERE/player.py org.mpris.MediaPlayer2.stub1 $S/p1.log $S/f1 Playing >$S/p1.out 2>&1 & PLAYERS="$!" ;;
  eight)   for n in 1 2 3 4 5 6 7 8; do $PY $HERE/player.py org.mpris.MediaPlayer2.stub$n $S/p$n.log $S/f$n Playing >$S/p$n.out 2>&1 & PLAYERS="$PLAYERS $!"; done ;;
  mpv)     [ -f $ROOT/tone.wav ] || $PY -c 'import wave; w=wave.open("'$ROOT'/tone.wav","wb"); w.setnchannels(1); w.setsampwidth(1); w.setframerate(8000); w.writeframes(bytes([128])*8000*600); w.close()'
           $MPV --no-config --idle=yes --force-window=no --ao=null --vo=null --no-terminal --force-media-title="Real Song" $ROOT/tone.wav >$S/mpv.log 2>&1 & PLAYERS="$!" ;;
  mpvlavfi) $MPV --no-config --idle=yes --force-window=no --ao=null --vo=null --no-terminal --force-media-title="Real Song" "av://lavfi:sine=frequency=440:duration=3600" >$S/mpv.log 2>&1 & PLAYERS="$!" ;;
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
kill $BAR $PLAYERS $DBD $SCOOT 2>/dev/null
sleep 0.5
echo "bar.log lines: $(wc -l < $S/bar.log)"; head -3 $S/bar.log
