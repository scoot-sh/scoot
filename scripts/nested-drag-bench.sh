#!/usr/bin/env bash
# A `--nested` drag-resize, driven for real and measured: what the nested
# scoot pays per distinct size its host resizes it through.
#
# The host is cage on wlroots' headless backend (pixman), the same host
# `smoke-test.sh`'s `--nested` run uses. cage keeps its one toplevel
# fullscreen, so resizing the host's *output* is resizing scoot's window:
# every `wlr-randr --custom-mode` step sends the nested scoot the
# `xdg_toplevel.configure` a drag would, one per step. A `foot` runs inside
# the nested session so there is a real client texture on screen across the
# drag.
#
# Reports, for the nested scoot only: CPU (utime+stime jiffies from
# /proc/<pid>/stat) over an idle baseline and over the drag, the number of
# sizes it actually applied (host configures are coalesced to one per frame
# tick, so steps can merge), and CPU per applied size. Then it stops at a few
# sizes and, once a frame at that size has drawn, takes an IPC screenshot of
# the nested session and a `grim` shot of the host, checking each PNG's
# dimensions. The nested scoot's RSS is printed before and after the drag, so
# a target that is not freed across resizes shows as growth.
#
#   SCOOT=/path/to/scoot RENDERER=gles STEPS=120 PACE=0.03 \
#       PREFIX=/tmp/drag-after scripts/nested-drag-bench.sh
#
# Needs cage, wlr-randr, foot and grim on PATH and a writable
# $XDG_RUNTIME_DIR. The per-resize lines it counts are debug-level: `resized
# the render target in place` (a GLES resize that kept its renderer), and
# `built another GLES renderer` / `rebuilt the GLES renderer at a new size`
# (a GLES resize that built a whole new renderer; the second is that line's
# wording before in-place resizing landed). pixman logs none of them, so
# under pixman both counts read 0 and only the CPU figures apply.
set -euo pipefail

SCOOT=${SCOOT:-${CARGO_TARGET_DIR:-target}/debug/scoot}
RENDERER=${RENDERER:-gles}
STEPS=${STEPS:-120}
PACE=${PACE:-0.03}
PREFIX=${PREFIX:-/tmp/drag}
if [ ! -x "$SCOOT" ]; then
    echo "no scoot binary at $SCOOT -- build one, or set SCOOT=/path/to/scoot" >&2
    exit 1
fi
for tool in cage wlr-randr foot grim od; do
    command -v "$tool" >/dev/null || { echo "$tool is not on PATH" >&2; exit 1; }
done
echo "using $SCOOT ($(date -r "$SCOOT" '+%Y-%m-%d %H:%M:%S' 2>/dev/null || echo 'mtime unknown')), renderer=$RENDERER steps=$STEPS pace=$PACE"

mkdir -p "$(dirname "$PREFIX")"
HOST_LOG="$PREFIX-cage.log"
INNER_SOCK="$PREFIX-inner.sock"
INNER_LOG="$PREFIX-inner.log"
HOST_DISPLAY_FILE="$PREFIX-host-display"
INNER_PID_FILE="$PREFIX-inner-pid"
rm -f "$HOST_LOG" "$INNER_SOCK" "$INNER_LOG" "$HOST_DISPLAY_FILE" "$INNER_PID_FILE" \
    "$PREFIX"-shot-*.png "$PREFIX"-host-*.png

host_pid=""
cleanup() {
    [ -n "$host_pid" ] && kill "$host_pid" 2>/dev/null || true
}
trap cleanup EXIT

strip() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }

# cage hands its WAYLAND_DISPLAY to the one application it runs; the shell
# writes that (for wlr-randr and grim) and its own pid (which `exec` keeps,
# so it is the nested scoot's) before becoming the nested scoot. The nested
# scoot's log goes to its own file, apart from cage's.
env WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    cage -- sh -c 'echo "$WAYLAND_DISPLAY" >"$0"; echo $$ >"$1"; exec env SCOOT_SOCKET= \
        RUST_LOG="info,scoot::compositor::headless=debug,scoot::compositor::render::gles=debug" \
        "$2" --nested --renderer "$3" --width 800 --height 600 --socket "$4" >"$5" 2>&1' \
    "$HOST_DISPLAY_FILE" "$INNER_PID_FILE" "$SCOOT" "$RENDERER" "$INNER_SOCK" "$INNER_LOG" \
    >"$HOST_LOG" 2>&1 &
host_pid=$!
for _ in $(seq 1 100); do [ -S "$INNER_SOCK" ] && break; sleep 0.1; done
[ -S "$INNER_SOCK" ] || { echo "the nested scoot never came up"; strip "$INNER_LOG" | tail -20; exit 1; }
HOST_DISPLAY=$(cat "$HOST_DISPLAY_FILE")
inner_pid=$(cat "$INNER_PID_FILE")
inner() { SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg "$@"; }
host() { env WAYLAND_DISPLAY="$HOST_DISPLAY" "$@"; }

inner action spawn foot >/dev/null
for _ in $(seq 1 50); do
    inner windows | grep -q '"app_id"' && break
    sleep 0.2
done
inner windows | grep -q '"app_id"' || { echo "foot never mapped inside"; exit 1; }
sleep 1

jiffies() { awk '{print $14 + $15}' "/proc/$inner_pid/stat"; }
rss() { awk '/^VmRSS:/ {print $2 " kB"}' "/proc/$inner_pid/status"; }
in_place() { strip "$INNER_LOG" | grep -c 'resized the render target in place' || true; }
rebuilt() {
    strip "$INNER_LOG" \
        | grep -cE 'built another GLES renderer|rebuilt the GLES renderer at a new size' || true
}
inner_size() {
    inner outputs | tr -d ' \n' \
        | grep -o '"rect":{"x":0,"y":0,"width":[0-9]*,"height":[0-9]*}' \
        | head -1 | grep -o '[0-9]*' | tail -2 | paste -sd'x'
}
# A PNG's IHDR width and height: big-endian u32s at bytes 16..24.
png_size() {
    od -An -tu1 -j16 -N8 "$1" \
        | awk '{ printf "%dx%d\n", (($1*256+$2)*256+$3)*256+$4, (($5*256+$6)*256+$7)*256+$8 }'
}
resize_host() {
    host wlr-randr --output HEADLESS-1 --custom-mode "$1x$2"
}

hz=$(getconf CLK_TCK)
idle_start=$(jiffies); sleep 3; idle=$(( $(jiffies) - idle_start ))
echo "idle: $idle jiffies over 3 s (CLK_TCK=$hz, inner size $(inner_size), RSS $(rss))"

before_in_place=$(in_place); before_rebuilt=$(rebuilt)
start=$(jiffies); started=$(date +%s.%N)
for i in $(seq 1 "$STEPS"); do
    resize_host $((800 + i * 5)) $((600 + i * 3))
    sleep "$PACE"
done
ended=$(date +%s.%N)
sleep 1
spent=$(( $(jiffies) - start ))
applied_in_place=$(( $(in_place) - before_in_place ))
applied_rebuilt=$(( $(rebuilt) - before_rebuilt ))
applied=$(( applied_in_place + applied_rebuilt ))
wall=$(awk -v a="$started" -v b="$ended" 'BEGIN { printf "%.2f", b - a }')
echo "drag: $STEPS steps in ${wall}s; sizes applied: $applied_in_place in place, $applied_rebuilt by a new GLES renderer; $spent jiffies (1 s settle included; idle baseline $idle per 3 s)"
if [ "$applied" -gt 0 ]; then
    awk -v j="$spent" -v n="$applied" -v hz="$hz" \
        'BEGIN { printf "drag: %.2f ms of nested-scoot CPU per applied size\n", j * 1000 / hz / n }'
fi
echo "inner size after the drag: $(inner_size), RSS $(rss)"

n=0
for size in 1024x700 700x900 1500x950; do
    n=$((n + 1))
    resize_host "${size%x*}" "${size#*x}"
    sleep 0.7
    shot="$PREFIX-shot-$n-$size.png"
    inner screenshot --out "$shot" >/dev/null
    host_shot="$PREFIX-host-$n-$size.png"
    host grim "$host_shot"
    echo "shot $n: inner $shot $(png_size "$shot"), host $host_shot $(png_size "$host_shot") (asked $size, inner output $(inner_size))"
done
echo "log: $INNER_LOG"
strip "$INNER_LOG" | grep -iE "warn|error" | grep -v "MESA" | head -5 || true
