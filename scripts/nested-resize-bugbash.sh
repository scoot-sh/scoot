#!/usr/bin/env bash
# Bug bash for the `--nested` follow-the-host-resize path (issue #144).
# Same outer-scoot-as-host driver as nested-resize-repro.sh, but past the
# happy path: a client mapped inside the nested session, resizes back to back
# with no settle time, same-size configures from host focus churn, and the
# mode list a capture/randr client sees.
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
PREFIX=${PREFIX:-/tmp/bb}
OUTER_SOCK="$PREFIX-outer.sock"
INNER_SOCK="$PREFIX-inner.sock"
OUTER_LOG="$PREFIX-outer.log"
INNER_LOG="$PREFIX-inner.log"

rm -f "$OUTER_SOCK" "$INNER_SOCK" "$OUTER_LOG" "$INNER_LOG"
outer_pid=""; inner_pid=""
cleanup() {
    [ -n "$inner_pid" ] && kill "$inner_pid" 2>/dev/null || true
    [ -n "$outer_pid" ] && kill "$outer_pid" 2>/dev/null || true
    pkill -f "$PREFIX-foot" 2>/dev/null || true
}
trap cleanup EXIT

strip() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }

wait_for_socket() {
    for _ in $(seq 1 100); do [ -S "$1" ] && return 0; sleep 0.1; done
    echo "socket $1 never appeared"; strip "$2" | tail -20; exit 1
}

inner_size() {
    SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg outputs \
        | tr -d ' \n' | grep -o '"rect":{"x":0,"y":0,"width":[0-9]*,"height":[0-9]*}' \
        | head -1 | grep -o '[0-9]*' | tail -2 | paste -sd'x'
}

"$SCOOT" --headless --width 1200 --height 800 --socket "$OUTER_SOCK" >"$OUTER_LOG" 2>&1 &
outer_pid=$!
wait_for_socket "$OUTER_SOCK" "$OUTER_LOG"
outer_display=$(strip "$OUTER_LOG" | grep -o 'wayland="[^"]*"' | head -1 | cut -d'"' -f2)

env WAYLAND_DISPLAY="$outer_display" SCOOT_SOCKET= \
    "$SCOOT" --nested --width 1280 --height 800 --socket "$INNER_SOCK" >"$INNER_LOG" 2>&1 &
inner_pid=$!
wait_for_socket "$INNER_SOCK" "$INNER_LOG"
sleep 1
inner_display=$(strip "$INNER_LOG" | grep -o 'wayland="[^"]*"' | head -1 | cut -d'"' -f2)
echo "outer=$outer_display inner=$inner_display inner size=$(inner_size)"

echo
echo "=== 1. a client mapped INSIDE the nested session follows the resize ==="
SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg action spawn foot >/dev/null
for _ in $(seq 1 50); do
    SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg windows | grep -q '"app_id"' && break
    sleep 0.2
done
echo "inner window rect before: $(SCOOT_SOCKET=$INNER_SOCK $SCOOT msg windows | tr -d ' \n' | grep -o '"width":[0-9]*,"height":[0-9]*' | head -1)"
SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action cycle-column-width >/dev/null
sleep 1
echo "inner output after:       $(inner_size)"
echo "inner window rect after:  $(SCOOT_SOCKET=$INNER_SOCK $SCOOT msg windows | tr -d ' \n' | grep -o '"width":[0-9]*,"height":[0-9]*' | head -1)"

echo
echo "=== 2. twelve resizes back to back, no settle time ==="
for _ in $(seq 1 12); do
    SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action cycle-column-width >/dev/null
done
sleep 2
echo "outer window rect: $(SCOOT_SOCKET=$OUTER_SOCK $SCOOT msg windows | tr -d ' \n' | grep -o '"width":[0-9]*,"height":[0-9]*' | head -1)"
echo "inner output size: $(inner_size)"
if kill -0 "$inner_pid" 2>/dev/null; then echo "ok: inner still alive"; else echo "BUG: inner died"; fi

echo
echo "=== 3. screenshot at the final size ==="
SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg screenshot --out "$PREFIX-shot.png" >/dev/null
od -An -tu1 -j16 -N8 "$PREFIX-shot.png" \
    | awk '{print "screenshot: " $1*16777216+$2*65536+$3*256+$4 "x" $5*16777216+$6*65536+$7*256+$8}'

echo
echo "=== 4. same-size configures (host focus churn) must not grow the mode list ==="
modes_before=$(WAYLAND_DISPLAY="$inner_display" wlr-randr 2>/dev/null | grep -c "px," || true)
echo "inner modes before churn: $modes_before"
SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action spawn foot >/dev/null
sleep 2
for _ in $(seq 1 6); do
    SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action focus-column left >/dev/null
    SCOOT_SOCKET="$OUTER_SOCK" "$SCOOT" msg action focus-column right >/dev/null
done
sleep 1
modes_after=$(WAYLAND_DISPLAY="$inner_display" wlr-randr 2>/dev/null | grep -c "px," || true)
echo "inner modes after churn:  $modes_after"
echo "inner output size:        $(inner_size)"

echo
echo "=== 5. host goes away under the nested session ==="
kill "$outer_pid"; outer_pid=""
sleep 2
if kill -0 "$inner_pid" 2>/dev/null; then
    echo "inner still running after the host died (pid $inner_pid)"
else
    echo "inner exited with the host"
fi

echo
echo "=== inner log (non-startup lines) ==="
strip "$INNER_LOG" | grep -v "cursor theme\|dmabuf feedback\|scoot is up\|Initializing a xkbcommon\|Loaded Keymap\|Creating new" || echo "(none)"
