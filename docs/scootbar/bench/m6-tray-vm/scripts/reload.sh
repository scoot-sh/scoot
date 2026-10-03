#!/bin/bash
BIN=/tmp/tray-takeover-run/scootbar-tray
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/tray-takeover-run/reload; rm -rf $R; mkdir -p $R/out; chmod 700 $R
export XDG_RUNTIME_DIR=$R; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 800 --height 200 --socket $R/scoot.sock --config /dev/null >$R/out/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))
dbus-daemon --session --nofork --address=unix:path=$R/out/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $R/out/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$R/out/bus
printf 'left = ["tray"]\nright = ["clock"]\n' > $R/out/bar.toml
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $R/out/bar.toml >$R/out/bar.log 2>&1 &
BAR=$!
sleep 1.5
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-300-1 $R/out/i1.log $R/out/f1 0 >$R/out/i1.out 2>&1 &
I1=$!
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-300-2 $R/out/i2.log $R/out/f2 60 >$R/out/i2.out 2>&1 &
I2=$!
sleep 2.5
q() { WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | $PY -c 'import sys,json; d=json.loads(sys.stdin.read()); t=[m for m in d["modules"] if m["id"]=="tray"][0]; v=t.get("value"); print(json.dumps({"watcher":v["watcher"],"items":[(i["id"].split("/")[0],i["shown"]) for i in v["items"]]} if v else None))'; }
owners() { busctl --user list 2>/dev/null | grep "StatusNotifierWatcher" | awk "{printf \"%s(pid %s) \", \$1, \$2}"; }
echo "before reload:"; q; echo "owners of the watcher name: $(owners)"
for n in 1 2 3; do WAYLAND_DISPLAY=$WD $BIN msg reload 2>&1 | head -1; sleep 1.5; echo "after reload $n:"; q; echo "owners of the watcher name: $(owners)"; done
echo "fds of the bar: $(ls /proc/$BAR/fd | wc -l)"
# a config change that moves the tray: right side, then reload
printf 'right = ["tray", "clock"]\n' > $R/out/bar.toml
WAYLAND_DISPLAY=$WD $BIN msg reload 2>&1 | head -1; sleep 1.5; echo "after moving the tray in the config:"; q; echo "owners: $(owners)"
echo "bar.log:"; cat $R/out/bar.log
kill $BAR $I1 $I2 $DBD $SCOOT 2>/dev/null
