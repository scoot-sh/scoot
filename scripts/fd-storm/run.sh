#!/usr/bin/env bash
# fd-storm: the accept-storm regression from the review of PR #241.
#
# K "parker" connections each park 36 x 28 = 1008 fds in the received-fd
# queue (under the wayland-backend fork's 1024 cap, so they stay connected),
# then a burst of N connections arrives in a few milliseconds while a
# round-trip monitor measures the worst wait an ordinary client sees.
#
# Before the fd table reading was cached (fd_pressure::table), every
# accepted connection re-read /proc/self/fd, linear in open fds: K=58, N=4000
# froze scoot for 35.6 s. With the cache it is one reading per ~20x its cost:
# the worst wait should be comparable to main's (~0.36 s at N=4000 on the dev
# VM), dominated by accepting N clients, not by observing the table.
#
# usage: scripts/fd-storm/run.sh SCOOT_BIN [K] [N]   (defaults K=58 N=4000)
# Needs cc, pkg-config and libwayland-client headers (the dev VM has them),
# and a hard RLIMIT_NOFILE of at least ~70000 for K=58 (scoot raises its soft
# limit to min(hard, 65536); `ulimit -Hn` shows yours). Prints the monitor's
# summary line: `LAT n=... max=...ms`. Every path derives from one prefix.
set -u
BIN=$1; K=${2:-58}; N=${3:-4000}
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${FD_STORM_PREFIX:-/tmp/fd-storm}; mkdir -p "$PREFIX"
cc -O2 -Wall -o "$PREFIX/storm" "$HERE/storm.c" || exit 1
cc -O2 -Wall -o "$PREFIX/park" "$HERE/park.c" || exit 1
cc -O2 -Wall -o "$PREFIX/lat" "$HERE/lat.c" $(pkg-config --cflags --libs wayland-client) || exit 1
export XDG_RUNTIME_DIR=$PREFIX/xdg; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
: > "$PREFIX/empty.toml"
LOG=$PREFIX/scoot.log
RUST_LOG=scoot=info "$BIN" --headless --config "$PREFIX/empty.toml" --socket "$XDG_RUNTIME_DIR/ipc.sock" > "$LOG" 2>&1 &
PID=$!
CHILDREN=()
cleanup() { for c in "${CHILDREN[@]}"; do kill "$c" 2>/dev/null; done; kill -TERM $PID 2>/dev/null; wait $PID 2>/dev/null; rm -rf "$XDG_RUNTIME_DIR"; }
trap cleanup EXIT
for _ in $(seq 100); do grep -q "scoot is up" "$LOG" 2>/dev/null && break; sleep 0.1; done
export WAYLAND_DISPLAY=$(grep "scoot is up" "$LOG" | grep -o "wayland-[0-9]*" | head -1)
fds() { ls /proc/$PID/fd 2>/dev/null | wc -l; }
echo "scoot=$BIN soft=$(awk '/Max open files/{print $4}' /proc/$PID/limits) parkers=$K storm=$N idle_fds=$(fds)"
for i in $(seq "$K"); do "$PREFIX/park" 36 150 28 > "$PREFIX/park$i" 2>&1 & CHILDREN+=($!); sleep 0.15; done
for _ in $(seq 300); do [ "$(fds)" -ge $((18 + K * 1009 - 50)) ] && break; sleep 0.2; done
echo "parked fds=$(fds)"
"$PREFIX/lat" 25 > "$PREFIX/lat.out" 2>&1 & LPID=$!
sleep 3
"$PREFIX/storm" "$N" 30 > "$PREFIX/storm.out" 2>&1 & CHILDREN+=($!)
sleep 1; cat "$PREFIX/storm.out"
wait $LPID
tail -1 "$PREFIX/lat.out"
echo "fds at end=$(fds) alive=$(kill -0 $PID 2>/dev/null && echo yes || echo no) shed=$(grep -c 'shed a pending' "$LOG")"
