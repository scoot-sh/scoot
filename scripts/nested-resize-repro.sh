#!/usr/bin/env bash
# Issue #144 repro / verification, with no webtop and no interactive drag.
#
# `cage` cannot resize its toplevel, so `MODE=--nested scripts/smoke-test.sh`
# can only prove steady state. This drives a *real* host-side resize instead:
# an outer scoot is the host compositor, an inner `--nested` scoot is one of
# its clients, and `action cycle-column-width` on the outer makes it configure
# that client to a different width -- the same `xdg_toplevel::Configure` a
# browser resize produces under Selkies/pixelflux.
#
# Reports `scoot msg outputs` from the inner session before and after.
set -euo pipefail

# Defaults to the tree you invoked from, deliberately: an absolute shared
# path here is how `smoke-test.sh` handed three different agents someone
# else's binary in one session (see
# docs/backlog/testing/smoke-test-binary-default.md). Override with SCOOT=.
SCOOT=${SCOOT:-${CARGO_TARGET_DIR:-target}/debug/scoot}
if [ ! -x "$SCOOT" ]; then
    echo "no scoot binary at $SCOOT -- build one, or set SCOOT=/path/to/scoot" >&2
    exit 1
fi
echo "using $SCOOT ($(date -r "$SCOOT" '+%Y-%m-%d %H:%M' 2>/dev/null || echo 'mtime unknown'))"
PREFIX=${PREFIX:-/tmp/wt}
OUTER_SOCK="$PREFIX-outer.sock"
INNER_SOCK="$PREFIX-inner.sock"
OUTER_LOG="$PREFIX-outer.log"
INNER_LOG="$PREFIX-inner.log"

rm -f "$OUTER_SOCK" "$INNER_SOCK" "$OUTER_LOG" "$INNER_LOG"

outer_pid=""
inner_pid=""
cleanup() {
    [ -n "$inner_pid" ] && kill "$inner_pid" 2>/dev/null || true
    [ -n "$outer_pid" ] && kill "$outer_pid" 2>/dev/null || true
}
trap cleanup EXIT

wait_for_socket() {
    local socket=$1 log=$2
    for _ in $(seq 1 100); do
        [ -S "$socket" ] && return 0
        sleep 0.1
    done
    echo "the control socket $socket never appeared; log:"
    tail -20 "$log"
    exit 1
}

outputs() {
    SCOOT_SOCKET=$1 "$SCOOT" msg outputs
}

echo "=== outer host compositor (headless 1200x800) ==="
"$SCOOT" --headless --width 1200 --height 800 --socket "$OUTER_SOCK" \
    >"$OUTER_LOG" 2>&1 &
outer_pid=$!
wait_for_socket "$OUTER_SOCK" "$OUTER_LOG"
# The log is ANSI-coloured, so strip the escapes before matching.
outer_display=$(sed 's/\x1b\[[0-9;]*m//g' "$OUTER_LOG" \
    | grep -o 'wayland="[^"]*"' | head -1 | cut -d'"' -f2 || true)
if [ -z "$outer_display" ]; then
    echo "could not read the outer compositor's WAYLAND_DISPLAY; log:"
    sed 's/\x1b\[[0-9;]*m//g' "$OUTER_LOG" | tail -5
    exit 1
fi
echo "outer WAYLAND_DISPLAY=$outer_display"

echo "=== inner nested compositor (--nested 1280x800, inside the outer) ==="
env WAYLAND_DISPLAY="$outer_display" SCOOT_SOCKET= \
    "$SCOOT" --nested --width 1280 --height 800 --socket "$INNER_SOCK" \
    >"$INNER_LOG" 2>&1 &
inner_pid=$!
wait_for_socket "$INNER_SOCK" "$INNER_LOG"

# Let the first configure land and the first frame present.
sleep 1
echo "--- inner outputs, after the host's FIRST configure ---"
outputs "$INNER_SOCK"
echo "--- outer's view of its one window ---"
SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg windows

for round in 1 2 3; do
    echo "=== host resize $round: cycle-column-width on the outer ==="
    SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action cycle-column-width
    sleep 1
    echo "--- outer's window rect for the nested scoot ---"
    SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg windows
    echo "--- inner outputs ---"
    outputs "$INNER_SOCK"
done

echo "=== inner still alive and serving IPC? ==="
if kill -0 "$inner_pid" 2>/dev/null; then
    echo "ok: the inner compositor is still running (pid $inner_pid)"
else
    echo "BUG: the inner compositor exited"
fi
SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg version

echo "=== inner screenshot at the final size ==="
SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg screenshot --out "$PREFIX-inner.png"
ls -l "$PREFIX-inner.png"

echo "=== inner log ==="
sed 's/\x1b\[[0-9;]*m//g' "$INNER_LOG"
