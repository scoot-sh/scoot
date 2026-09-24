#!/usr/bin/env bash
# `--nested --renderer gles`: what presenting to the host costs, read-back
# into wl_shm against dma-buf (nested/gpu.rs), under the same load.
#
# The host is an outer `scoot --headless --renderer gles` rather than cage:
# on the dev VM cage's gles2 renderer has no output (its swapchain cannot
# allocate on the render node) and its pixman renderer advertises no
# linux-dmabuf, so neither can take a dma-buf frame; an outer scoot can, and
# its IPC screenshot is the "host grim". The outer scoot is configured so the
# nested window covers its whole output (no gap, no ring, one full-width
# column), which makes the two screenshots directly comparable.
#
# Which path the nested scoot takes is the build: a default build presents by
# read-back, a `--features gpu-scanout` build by dma-buf where the host and
# renderer allow (its log says which, once, at INFO). Run it once per binary,
# same commit, same host binary:
#
#   SCOOT=/path/default/scoot  PREFIX=/tmp/ndb-readback scripts/nested-dmabuf-bench.sh
#   SCOOT=/path/gpu/scoot      PREFIX=/tmp/ndb-dmabuf   scripts/nested-dmabuf-bench.sh
#
# Scenes, each measured as utime+stime jiffies of the nested scoot *and* the
# host (/proc/<pid>/stat), after an idle baseline:
#
#   1. frames: a foot inside the nested session prints a counter at RATE Hz
#      for SECS seconds -- one small client commit per tick, which the nested
#      scoot composites (whole frame: `--nested` renders at buffer age 0) and
#      presents. Normalised per client tick.
#   2. resizes: the host steps the nested window through STEPS distinct
#      widths (the host's column_widths list, `set-column-width N`), PACE
#      apart, CYCLES times over. Normalised per size the nested scoot applied
#      (its debug line `resized the render target in place`; under dma-buf
#      also how many first frames at a new size went out without a second
#      draw, `handed the owed frame ...`). RSS and open fd
#      count of both processes before and after, so a host buffer chain that
#      is not freed across resizes shows as growth.
#   3. pixels: a screenshot of the nested session and one of the host, which
#      must be byte-identical (same size, same content, same PNG encoder).
#
# Needs foot on PATH and a writable $XDG_RUNTIME_DIR. HOST_SCOOT defaults to
# SCOOT; use one host binary for both runs.
set -euo pipefail

SCOOT=${SCOOT:-${CARGO_TARGET_DIR:-target}/debug/scoot}
HOST_SCOOT=${HOST_SCOOT:-$SCOOT}
PREFIX=${PREFIX:-/tmp/ndb}
W=${W:-1024}
H=${H:-768}
RATE=${RATE:-30}
SECS=${SECS:-20}
STEPS=${STEPS:-40}
PACE=${PACE:-0.05}
CYCLES=${CYCLES:-3}
for binary in "$SCOOT" "$HOST_SCOOT"; do
    [ -x "$binary" ] || { echo "no scoot binary at $binary" >&2; exit 1; }
done
command -v foot >/dev/null || { echo "foot is not on PATH" >&2; exit 1; }
echo "nested $SCOOT ($(date -r "$SCOOT" '+%F %T')), host $HOST_SCOOT ($(date -r "$HOST_SCOOT" '+%F %T'))"
echo "rate=$RATE secs=$SECS steps=$STEPS pace=$PACE cycles=$CYCLES size=${W}x$H"

mkdir -p "$(dirname "$PREFIX")"
HOST_SOCK="$PREFIX-host.sock"
INNER_SOCK="$PREFIX-inner.sock"
HOST_LOG="$PREFIX-host.log"
INNER_LOG="$PREFIX-inner.log"
INNER_PID_FILE="$PREFIX-inner-pid"
rm -f "$HOST_SOCK" "$INNER_SOCK" "$HOST_LOG" "$INNER_LOG" "$INNER_PID_FILE" \
    "$PREFIX"-*.png "$PREFIX"-*.toml

# 40 widths between half and all of the host's output, then the full width
# last (index $STEPS) so the pixel scene compares like with like.
widths=$(awk -v n="$STEPS" 'BEGIN { for (i = 0; i < n; i++) printf "%s%.4f", (i ? ", " : ""), 0.5 + 0.49 * i / n; printf ", 1.0" }')
cat >"$PREFIX-host.toml" <<TOML
[layout]
gap = 0
column_widths = [$widths]
default_column_width = $STEPS
[appearance]
focus_ring_width = 0
background_color = "#203040"
TOML
cat >"$PREFIX-inner.toml" <<TOML
[appearance]
background_color = "#406020"
TOML

host_pid=""
cleanup() {
    [ -n "$host_pid" ] && kill "$host_pid" 2>/dev/null || true
}
trap cleanup EXIT

strip() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }

env -u WAYLAND_DISPLAY SCOOT_SOCKET= RUST_LOG=info "$HOST_SCOOT" --headless --renderer gles \
    --width "$W" --height "$H" --socket "$HOST_SOCK" --config "$PREFIX-host.toml" -- \
    sh -c 'echo $$ >"$0"; exec env SCOOT_SOCKET= \
        RUST_LOG="info,scoot::compositor::headless=debug,scoot::compositor::nested=debug" \
        "$1" --nested --renderer gles --width 800 --height 600 --socket "$2" --config "$3" >"$4" 2>&1' \
    "$INNER_PID_FILE" "$SCOOT" "$INNER_SOCK" "$PREFIX-inner.toml" "$INNER_LOG" \
    >"$HOST_LOG" 2>&1 &
host_pid=$!
for _ in $(seq 1 100); do [ -S "$INNER_SOCK" ] && break; sleep 0.1; done
[ -S "$INNER_SOCK" ] || { echo "the nested scoot never came up"; strip "$INNER_LOG" | tail -20; exit 1; }
inner_pid=$(cat "$INNER_PID_FILE")
inner() { SCOOT_SOCKET="$INNER_SOCK" "$SCOOT" msg "$@"; }
host() { SCOOT_SOCKET="$HOST_SOCK" "$HOST_SCOOT" msg "$@"; }

echo "presentation: $(strip "$INNER_LOG" | grep -oE 'nested: presenting to the host by [^=]*(reason=.*)?' | head -1)"

jiffies() { awk '{print $14 + $15}' "/proc/$1/stat"; }
rss() { awk '/^VmRSS:/ {print $2 " kB"}' "/proc/$1/status"; }
fds() { ls "/proc/$1/fd" | wc -l; }
applied() { strip "$INNER_LOG" | grep -c 'resized the render target in place' || true; }
hz=$(getconf CLK_TCK)
per() { awk -v j="$1" -v n="$2" -v hz="$hz" 'BEGIN { if (n > 0) printf "%.3f", j * 1000 / hz / n; else print "n/a" }'; }
# A PNG's IHDR width and height: big-endian u32s at bytes 16..24.
png_size() {
    od -An -tu1 -j16 -N8 "$1" \
        | awk '{ printf "%dx%d\n", (($1*256+$2)*256+$3)*256+$4, (($5*256+$6)*256+$7)*256+$8 }'
}

# The counter client: one commit per tick, a few glyphs of damage each.
inner action spawn foot sh -c "i=0; while :; do i=\$((i+1)); printf '\\r%08d' \$i; sleep $(awk -v r="$RATE" 'BEGIN { printf "%.4f", 1 / r }'); done" >/dev/null
for _ in $(seq 1 50); do
    inner windows | grep -q '"app_id"' && break
    sleep 0.2
done
inner windows | grep -q '"app_id"' || { echo "foot never mapped inside"; exit 1; }
sleep 2

i0=$(jiffies "$inner_pid"); h0=$(jiffies "$host_pid"); sleep 5
idle_inner=$(( $(jiffies "$inner_pid") - i0 )); idle_host=$(( $(jiffies "$host_pid") - h0 ))
echo "warm-up (5 s, counter already running): nested $idle_inner jiffies, host $idle_host jiffies"

# Scene 1: SECS seconds of the counter.
i0=$(jiffies "$inner_pid"); h0=$(jiffies "$host_pid"); sleep "$SECS"
frames_inner=$(( $(jiffies "$inner_pid") - i0 )); frames_host=$(( $(jiffies "$host_pid") - h0 ))
ticks=$(( RATE * SECS ))
echo "frames: $SECS s at $RATE Hz (~$ticks client ticks): nested $frames_inner jiffies ($(per "$frames_inner" "$ticks") ms/tick), host $frames_host jiffies ($(per "$frames_host" "$ticks") ms/tick)"

# Scene 2: resizes.
echo "before resizes: nested RSS $(rss "$inner_pid") fds $(fds "$inner_pid"); host RSS $(rss "$host_pid") fds $(fds "$host_pid")"
before=$(applied)
i0=$(jiffies "$inner_pid"); h0=$(jiffies "$host_pid")
for _ in $(seq 1 "$CYCLES"); do
    for index in $(seq 0 $((STEPS - 1))); do
        host action set-column-width "$index" >/dev/null
        sleep "$PACE"
    done
done
host action set-column-width "$STEPS" >/dev/null
sleep 1
resize_inner=$(( $(jiffies "$inner_pid") - i0 )); resize_host=$(( $(jiffies "$host_pid") - h0 ))
sizes=$(( $(applied) - before ))
handed=$(strip "$INNER_LOG" | grep -c 'handed the owed frame to the host without redrawing it' || true)
echo "resizes: $((CYCLES * STEPS + 1)) steps, $sizes sizes applied: nested $resize_inner jiffies ($(per "$resize_inner" "$sizes") ms/size, counter still running), host $resize_host jiffies; owed frames handed over without a redraw so far: $handed"
echo "after resizes: nested RSS $(rss "$inner_pid") fds $(fds "$inner_pid"); host RSS $(rss "$host_pid") fds $(fds "$host_pid")"

# Scene 3: pixels. Stop the counter first so both shots see one frame.
inner action close >/dev/null
sleep 1
inner action spawn foot >/dev/null
sleep 3
inner screenshot --no-cursor --out "$PREFIX-inner.png" >/dev/null
host screenshot --no-cursor --out "$PREFIX-host.png" >/dev/null
if cmp -s "$PREFIX-inner.png" "$PREFIX-host.png"; then verdict=identical; else verdict=DIFFERENT; fi
echo "pixels: nested $PREFIX-inner.png $(png_size "$PREFIX-inner.png"), host $PREFIX-host.png $(png_size "$PREFIX-host.png"): $verdict"
echo "logs: $INNER_LOG $HOST_LOG"
# Only the EGL device probe's own errors are filtered (virtio's DRI2 screen
# refusal before kms_swrast, on every GLES start); anything else is shown.
strip "$INNER_LOG" | grep -E " (WARN|ERROR) " | grep -v "smithay::backend::egl::ffi" | head -5 || true
