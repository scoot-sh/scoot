#!/bin/bash
# Two scoot --headless sessions, one scootbar (tray) each, ONE private dbus-daemon.
# A starts first and owns the watcher; B hosts against it. Then: an item
# registers after both are up; A is kill -9'd; B takes the name and keeps the item;
# a second item registers with B as the owner.
BIN=/tmp/tray-takeover-run/scootbar-tray
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
T=/tmp/tray-takeover-run/two; rm -rf $T; mkdir -p $T/a $T/b $T/out; chmod 700 $T/a $T/b
printf 'left = ["tray"]\n' > $T/bar.toml
dbus-daemon --session --nofork --address=unix:path=$T/out/bus >/dev/null 2>&1 &
DBD=$!
for i in $(seq 1 100); do [ -S $T/out/bus ] && break; sleep 0.1; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$T/out/bus
start() { # name -> sets WD_<name>, PIDs
  local n=$1 R=$T/$1
  XDG_RUNTIME_DIR=$R /var/cargo-target/release/scoot --headless --outputs 1 --width 800 --height 200 --socket $R/scoot.sock --config /dev/null >$T/out/scoot-$n.log 2>&1 &
  eval "SCOOT_$n=$!"
  for i in $(seq 1 100); do ls $R/wayland-* >/dev/null 2>&1 && [ -S $R/scoot.sock ] && break; sleep 0.1; done
  eval "WD_$n=$(basename $(ls $R/wayland-* | grep -v '\.lock' | head -1))"
}
start a; start b
bar() { local n=$1; local wd; eval "wd=\$WD_$n"; XDG_RUNTIME_DIR=$T/$n WAYLAND_DISPLAY=$wd $BIN daemon --font $FONT --config $T/bar.toml >$T/out/bar-$n.log 2>&1 & eval "BAR_$n=$!"; }
q() { local n=$1 wd; eval "wd=\$WD_$n"; XDG_RUNTIME_DIR=$T/$n WAYLAND_DISPLAY=$wd $BIN msg query 2>&1 | $PY -c 'import sys,json; d=json.loads(sys.stdin.read()); t=[m for m in d["modules"] if m["id"]=="tray"][0]; v=t.get("value"); print(json.dumps({"watcher":v["watcher"],"items":[(i["id"].split("/")[0],i["title"],i["shown"]) for i in v["items"]]} if v else None))'; }
bar a; sleep 1.5; bar b; sleep 2
echo "--- 1. both bars up, no items: A owns, B hosts (empty tray is no value)"; q a; q b
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-1-1 $T/out/i1.log $T/out/f1 0 >$T/out/i1.out 2>&1 &
I1=$!
sleep 2
echo "--- 2. an item registers after both are up: shown in BOTH"; q a; q b
echo "--- 3. kill -9 bar A (the watcher owner)"
kill -9 $BAR_a; sleep 1.5
echo "B after A died:"; q b
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems
busctl --user list 2>/dev/null | grep -i "StatusNotifierWatcher"
$PY /tmp/tray-takeover-run/item.py org.kde.StatusNotifierItem-1-2 $T/out/i2.log $T/out/f2 60 >$T/out/i2.out 2>&1 &
I2=$!
sleep 2
echo "--- 4. a second item registers with B as owner"; q b
echo "--- 5. bar A restarts: hosts against B, lists both"
bar a; sleep 2.5; q a; q b
echo "--- 6. kill -9 item 1: gone from both"
kill -9 $I1; sleep 1.2; q a; q b
echo "bar-a.log:"; cat $T/out/bar-a.log; echo "bar-b.log:"; cat $T/out/bar-b.log
kill $BAR_a $BAR_b $I2 $DBD $SCOOT_a $SCOOT_b 2>/dev/null
