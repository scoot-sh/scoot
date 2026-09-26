#!/usr/bin/env bash
# popup-flood: the side-by-side popup burst from
# docs/backlog/core/popup-count-quadratic.md.
#
# One headless scoot serves N-popups floods from a single client: every
# get_popup with no commits and a round trip (phase "track": admit plus the
# tree insert), every first commit and a round trip (phase "commit": the
# commit scan plus the initial configure walk), every destroy and a round
# trip (phase "destroy": the xdg_popup destructor scan). Each round trip
# waits out the server's dispatch, so each phase time is the server's stall
# for it, plus the client's own send/receive.
#
# No buffers: popup surfaces commit empty (the stall is in tracking, not
# drawing), and thousands of shm pools would trip the client's own 512-pool
# bound long before the popup count mattered. The window gets one small
# buffer so it is properly mapped. Past the per-client popup cap the client
# is refused instead: the flood prints REFUSED with the phase and the time
# to refuse it, and the exit is 0 -- this measures; it asserts nothing.
#
# usage: scripts/popup-flood/run.sh SCOOT_BIN ["N ..."]   (default "128 500 2000")
# Needs cc, pkg-config, wayland-scanner and libwayland-client headers (the
# dev VM has them). Prints one line per run; take the minimum of the three
# runs per N (noise only ever adds time). Every path derives from one prefix.
set -u
BIN=$1; NS=${2:-"128 500 2000"}
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${POPUP_FLOOD_PREFIX:-/tmp/popup-flood}; mkdir -p "$PREFIX"
PROTO_DATADIR=$(pkg-config --variable=pkgdatadir wayland-protocols)
wayland-scanner client-header "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-protocol.c" || exit 1
cc -O2 -Wall -o "$PREFIX/popup-flood" "$HERE/popup-flood.c" \
  "$PREFIX/xdg-shell-protocol.c" -I"$PREFIX" $(pkg-config --cflags --libs wayland-client) || exit 1
export XDG_RUNTIME_DIR=$PREFIX/xdg-run; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
: > "$PREFIX/empty.toml"
LOG=$PREFIX/scoot.log
"$BIN" --headless --config "$PREFIX/empty.toml" > "$LOG" 2>&1 &
PID=$!
cleanup() { kill -TERM $PID 2>/dev/null; wait $PID 2>/dev/null; }
trap cleanup EXIT
for _ in $(seq 200); do
  SOCK=$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E "^wayland-[0-9]+$" | head -1)
  [ -n "${SOCK:-}" ] && break
  sleep 0.1
done
export WAYLAND_DISPLAY=$SOCK
echo "display=$WAYLAND_DISPLAY bin=$BIN"
sleep 5 # idle before measuring
for N in $NS; do
  for _ in 1 2 3; do
    sleep 3 # idle between runs; the minimum downstream
    "$PREFIX/popup-flood" "$N"
  done
done
