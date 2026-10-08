#!/bin/bash
# One item that re-announces NewIcon as fast as the bar re-reads it, for 20 s.
BIN=/tmp/tray-takeover-run/scootbar-tray
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/tray-takeover-run/flood; rm -rf $R; mkdir -p $R; chmod 700 $R
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 800 --height 200 --socket $R/scoot.sock --config /dev/null >$R/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
dbus-daemon --session --nofork --address=unix:path=$R/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $R/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$R/bus
printf 'left = ["tray"]\nright = ["clock"]\n' > $R/bar.toml
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $R/bar.toml >$R/bar.log 2>&1 &
BAR=$!
sleep 2
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-9-1 $R/i.log $R/flag 0 register flood >$R/i.out 2>&1 &
IT=$!
sleep 3
s() { echo "$1 rss_kb=$(awk '/VmRSS/{print $2}' /proc/$BAR/status) vol=$(awk '/^voluntary_ctxt/{print $2}' /proc/$BAR/status) nonvol=$(awk '/nonvoluntary_ctxt/{print $2}' /proc/$BAR/status) utime=$(awk '{print $14}' /proc/$BAR/stat) stime=$(awk '{print $15}' /proc/$BAR/stat) fds=$(ls /proc/$BAR/fd | wc -l) staged_ok=1"; }
s start
sleep 20
s end
echo "getalls answered: $(grep -c '^getall' $R/i.log)"
echo "bar alive: $(kill -0 $BAR 2>/dev/null && echo yes || echo no)"
# the scoot compositor's own cost for the same window is not the bar's: bar only.
WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | head -c 300; echo
kill $IT $BAR $DBD $SCOOT 2>/dev/null
