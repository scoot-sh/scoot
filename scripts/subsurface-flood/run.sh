#!/usr/bin/env bash
# subsurface-flood: the 1-deep sibling-subsurface burst from
# docs/backlog/core/subsurface-count-quadratic.md.
#
# One headless scoot serves desync and sync floods from a single client: N
# 1-deep 4x4-buffer sibling subsurfaces in one batch plus the window commit,
# and a round trip. The round trip waits out the server's dispatch, so the
# batch time is the server's stall for it, plus the client's own
# send/receive.
#
# Every subsurface attaches the same shared 4x4 shm buffer (the stall is in
# per-commit window work, not drawing), and the window gets one small buffer
# so it is properly mapped. This measures; it asserts nothing.
#
# usage: scripts/subsurface-flood/run.sh SCOOT_BIN ["N ..."]   (default "1000 3000 10000")
# Needs cc, pkg-config, wayland-scanner and libwayland-client headers (the
# dev VM has them). Prints one line per run; take the minimum of the three
# runs per N (noise only ever adds time). Every path derives from one prefix.
set -u
BIN=$1; NS=${2:-"1000 3000 10000"}
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${SUBSURFACE_FLOOD_PREFIX:-/tmp/subsurface-flood}; mkdir -p "$PREFIX"
PROTO_DATADIR=$(pkg-config --variable=pkgdatadir wayland-protocols)
wayland-scanner client-header "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-protocol.c" || exit 1
cc -O2 -Wall -o "$PREFIX/subsurface-flood" "$HERE/subsurface-flood.c" \
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
  for MODE in desync sync; do
    for _ in 1 2 3; do
      sleep 3 # idle between runs; the minimum downstream
      "$PREFIX/subsurface-flood" "$MODE" "$N"
    done
  done
done
