#!/bin/bash
# Live end-to-end: real scoot --headless, private dbus-daemon, scootbar (tray), two jeepney items.
BIN=/tmp/tray-takeover-run/scootbar-tray
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/tray-takeover-run/live; rm -rf $R; mkdir -p $R; chmod 700 $R
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 800 --height 200 --socket $R/scoot.sock --config /dev/null >$R/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
dbus-daemon --session --nofork --address=unix:path=$R/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $R/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$R/bus
mkdir $R/out
printf 'left = ["tray"]\nright = ["clock"]\n' > $R/bar.toml
# item 1 is up BEFORE the bar (found by ListNames), item 2 registers after.
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-100-1 $R/out/i1.log $R/out/flag1 0 noregister >$R/out/i1.out 2>&1 &
I1=$!
sleep 1
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $R/bar.toml >$R/out/bar.log 2>&1 &
BAR=$!
sleep 2
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-100-2 $R/out/i2.log $R/out/flag2 90 >$R/out/i2.out 2>&1 &
I2=$!
sleep 2
q() { WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | $PY -c 'import sys,json; d=json.loads(sys.stdin.read()); t=[m for m in d["modules"] if m["id"]=="tray"][0]; print(json.dumps(t.get("value")))'; }
echo "--- 1. two items shown (one found by enumeration, one registered)"; q
echo "--- sd-bus reads OUR watcher object (independent parser)"
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher IsStatusNotifierHostRegistered
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher ProtocolVersion
busctl --user introspect org.kde.StatusNotifierWatcher /StatusNotifierWatcher 2>&1 | head -12
echo "--- screenshot 1"
SCOOT_SOCKET=$R/scoot.sock /var/cargo-target/release/scoot msg screenshot --out $R/out/shot1.png 2>&1 | head -2
ls $R/out
echo "--- 2. click: scootbar msg invoke tray activate 0 / secondary 1 / wheel-up 1 / wheel-down 0 / menu 0"
for a in "activate 0" "secondary 1" "wheel-up 1" "wheel-down 0" "menu 0" "activate 9"; do WAYLAND_DISPLAY=$WD $BIN msg invoke tray $a 2>&1 | head -1; done
sleep 1
echo "item1 log:"; cat $R/out/i1.log; echo "item2 log:"; cat $R/out/i2.log
echo "--- 3. NewIcon: item 2 changes icon"
touch $R/out/flag2; sleep 1.5; tail -2 $R/out/i2.log
q
SCOOT_SOCKET=$R/scoot.sock /var/cargo-target/release/scoot msg screenshot --out $R/out/shot2.png 2>&1 | head -2
echo "--- 4. crash: kill -9 item 1 (no unregister)"
kill -9 $I1; sleep 1; q
SCOOT_SOCKET=$R/scoot.sock /var/cargo-target/release/scoot msg screenshot --out $R/out/shot3.png 2>&1 | head -2
echo "--- 5. watcher name taken by us; kill -9 item 2 -> empty"
kill -9 $I2; sleep 1; q
echo "bar.log:"; cat $R/out/bar.log
kill $BAR $DBD $SCOOT 2>/dev/null
