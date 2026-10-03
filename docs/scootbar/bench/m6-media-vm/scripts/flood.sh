#!/bin/bash
# One playing stub signalling as fast as its loop goes for 20 s: Position
# only (`flood`: a player that reports its position constantly) or a new
# title each time (`metaflood`: the worst a player can make the bar redraw).
# usage: flood.sh KIND(flood|metaflood|none) [BIN]
KIND=${1:-flood}; BIN=${2:-/tmp/media-bins/scootbar-on}
HERE=$(dirname "$0")
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/media-flood-$KIND; rm -rf $R; mkdir -p $R; chmod 700 $R
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 800 --height 200 --socket $R/scoot.sock --config /dev/null >$R/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
S=/tmp/media-flood-s-$KIND; rm -rf $S; mkdir -p $S
dbus-daemon --session --nofork --address=unix:path=$S/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $S/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$S/bus
printf 'left = ["media"]\n' > $S/bar.toml
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $S/bar.toml >$S/bar.log 2>&1 &
BAR=$!
$PY $HERE/player.py org.mpris.MediaPlayer2.stub1 $S/p.log $S/f Playing >$S/p.out 2>&1 &
P=$!
sleep 4
s() { echo "$1 rss_kb=$(awk '/VmRSS/{print $2}' /proc/$BAR/status) vol=$(awk '/^voluntary_ctxt/{print $2}' /proc/$BAR/status) nonvol=$(awk '/nonvoluntary_ctxt/{print $2}' /proc/$BAR/status) utime=$(awk '{print $14}' /proc/$BAR/stat) stime=$(awk '{print $15}' /proc/$BAR/stat) fds=$(ls /proc/$BAR/fd | wc -l)"; }
s before
[ "$KIND" != none ] && touch $S/f.$KIND
sleep 20
s after_20s
sleep 2
echo "player log tail:"; tail -3 $S/p.log
echo "bar alive: $(kill -0 $BAR 2>/dev/null && echo yes || echo no)"
echo "query: $(WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | head -c 400)"
echo "bar.log:"; cat $S/bar.log | head -5
kill $P $BAR $DBD $SCOOT 2>/dev/null
