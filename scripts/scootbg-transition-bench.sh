#!/bin/sh
# Resource numbers for the transitions ticket: idle CPU/RSS/wakeups/PSS
# after transitions complete vs instant sets, plus release binary size.
# One headless scoot output at 3840x2160, release builds.
#
# Usage: scripts/scootbg-transition-bench.sh <scoot> <scootbg>
set -eu

SCOOT="$1"
SCOOTBG="$2"
OUT="${3:-/tmp/trans-bench}"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/trans-bench-XXXXXX")"
trap 'kill "$scoot" "$daemon" 2>/dev/null || true; rm -rf "$scratch"' EXIT INT TERM
mkdir -p "$scratch/run" "$scratch/home"
chmod 700 "$scratch/run"
export XDG_RUNTIME_DIR="$scratch/run" HOME="$scratch/home" XDG_STATE_HOME="$scratch/home/state"
unset WAYLAND_DISPLAY WAYLAND_SOCKET SCOOT_SOCKET
: > "$scratch/empty.toml"

"$SCOOT" --headless --outputs 1 --width 3840 --height 2160 \
    --socket "$scratch/run/scoot.sock" --config "$scratch/empty.toml" \
    > "$scratch/scoot.log" 2>&1 &
scoot=$!
for i in $(seq 1 100); do
    if [ -S "$scratch/run/scoot.sock" ]; then
        break
    fi
    sleep 0.1
done
# The compositor names its Wayland socket; the daemon needs it.
for i in $(seq 1 100); do
    WAYLAND_DISPLAY="$(ls "$scratch/run" | grep '^wayland-' | head -1)"
    if [ -n "$WAYLAND_DISPLAY" ]; then
        break
    fi
    sleep 0.1
done
export WAYLAND_DISPLAY

"$SCOOTBG" daemon --profile bench > "$scratch/scootbg.log" 2>&1 &
daemon=$!
for i in $(seq 1 100); do "$SCOOTBG" query > /dev/null 2>&1 && break; sleep 0.1; done
pid() { echo "$daemon"; }

rss_kb() { awk '/^VmRSS:/{print $2}' "/proc/$(pid)/status"; }
pss_kb() { awk '/^Pss:/{print $2}' "/proc/$(pid)/smaps" 2>/dev/null | awk '{s+=$1} END {print s}'; }
wakeups() { grep -c . "/proc/$(pid)/task" 2>/dev/null; cat "/proc/$(pid)/status" | awk '/^voluntary_ctxt_switches:|^nonvoluntary_ctxt_switches:/{s+=$2} END {print s}'; }

idle_30s() {
    # $1 = label. Settles, then samples wakeups over 30 s.
    what="$1"
    sleep 2
    before=$(wakeups)
    start=$(date +%s)
    sleep 30
    after=$(wakeups)
    echo "$what rss_kb=$(rss_kb) pss_kb=$(pss_kb) switches_30s=$((after - before)) threads=$(wakeups | head -1)" | tee -a "$OUT.txt"
    echo "elapsed: $(( $(date +%s) - start ))s (want 30)" >&2
}

: > "$OUT.txt"
echo "== instant color ==" | tee -a "$OUT.txt"
"$SCOOTBG" set '#101014'
idle_30s "instant-color"
echo "== instant image ==" | tee -a "$OUT.txt"
"$SCOOTBG" set "$OUT.jpg" 2>/dev/null || "$SCOOTBG" set '#101014'
idle_30s "instant-image"
echo "== transitioned (3 fades) then idle ==" | tee -a "$OUT.txt"
"$SCOOTBG" set '#c03020' --transition fade --duration-ms 500
"$SCOOTBG" set '#101014' --transition wipe --angle 90 --duration-ms 500
"$SCOOTBG" set '#c03020' --transition grow --position 0.5,0.5 --duration-ms 500
idle_30s "after-transitions"
echo "== threads/fds ==" | tee -a "$OUT.txt"
echo "threads=$(ls /proc/$(pid)/task | wc -l) fds=$(ls /proc/$(pid)/fd | wc -l)" | tee -a "$OUT.txt"
"$SCOOTBG" kill
cat "$OUT.txt"
