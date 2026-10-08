#!/bin/bash
# Live end to end: real scoot --headless, a session bus, the real scootbar
# (release) with the media module, jeepney stub players (player.py) and real
# mpv instances (with its MPRIS script), playerctl as an independent client of
# the same players. usage: live.sh BUSKIND [BIN]
#   BUSKIND private: a private dbus-daemon; broker: the VM's own session bus
#   (dbus-broker), where only the names this script's players own are added.
KIND=${1:-private}
BIN=${2:-/tmp/media-bins/scootbar-on}
HERE=$(dirname "$0")
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
MPV=/nix/store/95shw2n7gm3d68zi6vkg19cdk5y5slrq-mpv-with-scripts-0.41.0/bin/mpv
PC=/nix/store/kw7x3hjc77jrgiq6476d03jxh7rws1qb-playerctl-2.4.1/bin/playerctl
FONT=$(ls /nix/store/*dejavu-fonts-minimal*/share/fonts/truetype/DejaVuSans.ttf | head -1)
R=/tmp/media-live-$KIND; rm -rf $R; mkdir -p $R; chmod 700 $R; mkdir $R/out
XRD=$R/xdg; mkdir -p $XRD; chmod 700 $XRD
export XDG_RUNTIME_DIR=$XRD; unset WAYLAND_DISPLAY
/var/cargo-target/release/scoot --headless --outputs 1 --width 900 --height 200 --socket $XRD/scoot.sock --config /dev/null >$R/scoot.log 2>&1 &
SCOOT=$!
for i in $(seq 1 100); do ls $XRD/wayland-* >/dev/null 2>&1 && [ -S $XRD/scoot.sock ] && break; sleep 0.1; done
WD=$(basename $(ls $XRD/wayland-* | grep -v '\.lock' | head -1))
DBD=
if [ "$KIND" = private ]; then
  dbus-daemon --session --nofork --address=unix:path=$R/bus >/dev/null 2>&1 &
  DBD=$!
  for i in $(seq 1 100); do [ -S $R/bus ] && break; sleep 0.1; done
  export DBUS_SESSION_BUS_ADDRESS=unix:path=$R/bus
else
  export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
  echo "bus owner: $(busctl --user status 2>&1 | grep -E 'BusScope|PID=' | tr '\n' ' ')"; ps -o args= -p $(busctl --user status 2>/dev/null | sed -n 's/^PID=//p') 2>/dev/null
  echo "mpris names already on the bus: $(busctl --user list --no-pager 2>/dev/null | grep -c org.mpris.MediaPlayer2)"
fi
printf 'left = ["media"]\nright = ["clock"]\n[media]\nmax-width = 300\n' > $R/bar.toml
sc() { SCOOT_SOCKET=$XRD/scoot.sock /var/cargo-target/release/scoot msg screenshot --out $R/out/$1.png 2>&1 | head -1; }
q() { WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | $PY -c 'import sys,json; d=json.loads(sys.stdin.read()); t=[m for m in d["modules"] if m["id"]=="media"][0]; print(json.dumps(t.get("value")), "| text:", json.dumps(t.get("text")))'; }
inv() { echo "invoke media $*: $(WAYLAND_DISPLAY=$WD $BIN msg invoke media "$@" 2>&1 | head -1)"; }
# A is up before the bar: found by listing the bus.
$PY $HERE/player.py org.mpris.MediaPlayer2.stub1 $R/out/a.log $R/out/fa Playing >$R/out/a.out 2>&1 &
A=$!
sleep 1
WAYLAND_DISPLAY=$WD $BIN daemon --font $FONT --config $R/bar.toml >$R/out/bar.log 2>&1 &
BAR=$!
sleep 2
echo "--- 1. one playing stub, up before the bar (found by ListNames)"; q; sc 1-playing
echo "--- playerctl (independent client) sees it too: $($PC -p stub1 status) / $($PC -p stub1 metadata xesam:title)"
echo "--- 2. a second stub appears paused: not shown while A plays"
$PY $HERE/player.py org.mpris.MediaPlayer2.stub2 $R/out/b.log $R/out/fb Paused >$R/out/b.out 2>&1 &
B=$!
sleep 1.5; q
echo "--- 3. controls: play-pause pauses A (the bar sends it, playerctl reads it back)"
inv play-pause; sleep 0.7; echo "A status by playerctl: $($PC -p stub1 status)"; q; sc 2-paused
echo "--- 4. next, after the skip gap"
sleep 0.4; inv next; sleep 0.7; q
echo "--- the same skip twice in a hurry is refused, with the reason"
inv next; inv previous
echo "A log:"; cat $R/out/a.log
echo "--- 5. B starts playing (playerctl does it): the most recent player is shown"
$PC -p stub2 play-pause; sleep 0.8; q; sc 3-second-player
echo "--- 6. A plays again, and its track becomes the long title (the stub sets it): cut with an ellipsis"
$PC -p stub1 play-pause; sleep 0.5; touch $R/out/fa.long; sleep 1; q; sc 4-long-title
echo "--- 7. kill -9 the shown player (its connection drops, the bus releases its name)"
kill -9 $A; sleep 1; q; sc 5-after-kill
echo "--- 8. kill -9 the other: nothing is shown"
kill -9 $B; sleep 1; q; sc 6-empty
echo "--- 9. refused with a reason now"
inv play-pause; inv seek; inv next 3
echo "--- 10. two real mpv instances (mpv's own MPRIS script)"
$MPV --no-config --idle=yes --force-window=no --ao=null --vo=null --no-terminal --force-media-title="Real One" "av://lavfi:sine=frequency=440:duration=3600" >$R/out/mpv1.log 2>&1 &
M1=$!
sleep 1.5
$MPV --no-config --idle=yes --force-window=no --ao=null --vo=null --no-terminal --force-media-title="Real Two" "av://lavfi:sine=frequency=880:duration=3600" >$R/out/mpv2.log 2>&1 &
M2=$!
sleep 2
echo "players on the bus: $($PC -l | tr '\n' ' ')"; q
sc 7-two-mpv
N1=mpv; N2=$($PC -l | grep instance | head -1)
echo "--- 11. the bar's play-pause reaches the player it shows, mpv reads it back"
shown=$(WAYLAND_DISPLAY=$WD $BIN msg query 2>&1 | $PY -c 'import sys,json; d=json.loads(sys.stdin.read()); t=[m for m in d["modules"] if m["id"]=="media"][0]; print(t["value"]["bus_name"].rsplit(".",2)[-1] if "instance" in t["value"]["bus_name"] else "mpv")')
echo "shown player: $shown"
inv play-pause; sleep 0.8
echo "statuses: mpv=$($PC -p mpv status) $N2=$($PC -p $N2 status)"; q; sc 8-mpv-paused
echo "--- 12. the other mpv is started from the outside: it becomes the one shown"
$PC -p $N2 play; sleep 0.5; $PC -p mpv play; sleep 0.8; q
kill $M1 $M2 2>/dev/null; sleep 1; echo "--- 13. both mpv gone:"; q
echo "bar.log:"; cat $R/out/bar.log
kill $BAR $DBD $SCOOT $A $B 2>/dev/null
