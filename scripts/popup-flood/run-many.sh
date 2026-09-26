#!/usr/bin/env bash
# popup-flood-many: the aggregate popup shape from
# docs/backlog/core/popup-aggregate-pressure-cap.md.
#
# One headless scoot serves K connections holding M side-by-side popups
# each (M at or under the per-client cap), every popup off its own
# connection's window, so the global PopupManager tree holds K*M popups
# with no client past its cap and nobody disconnected. Barriers separate
# the phases (track / commit / destroy), so each phase time is the
# server's stall for it, plus the clients' own send/receive.
#
# usage: scripts/popup-flood/run-many.sh SCOOT_BIN ["KxM ..."]   (default "10x128 20x128 40x128")
# Needs cc, pkg-config, wayland-scanner and libwayland-client headers (the
# dev VM has them). Prints one line per run; take the minimum of the three
# runs per shape (noise only ever adds time). Every path derives from one prefix.
set -u
BIN=$1; SHAPES=${2:-"10x128 20x128 40x128"}
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${POPUP_FLOOD_PREFIX:-/tmp/popup-flood}; mkdir -p "$PREFIX"
PROTO_DATADIR=$(pkg-config --variable=pkgdatadir wayland-protocols)
wayland-scanner client-header "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$PROTO_DATADIR/stable/xdg-shell/xdg-shell.xml" \
  "$PREFIX/xdg-shell-protocol.c" || exit 1
cc -O2 -Wall -o "$PREFIX/popup-flood-many" "$HERE/popup-flood-many.c" \
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
for SHAPE in $SHAPES; do
  K=${SHAPE%x*}; M=${SHAPE#*x}
  for _ in 1 2 3; do
    sleep 3 # idle between runs; the minimum downstream
    timeout 300 "$PREFIX/popup-flood-many" "$K" "$M"
  done
done
