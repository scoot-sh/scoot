#!/usr/bin/env bash
# A/B bench: scootlaunch (the release binary) vs fuzzel, on one scoot
# --headless session each. Measures launch-to-first-frame (cold/no-index
# and warm/indexed), keystroke-to-updated-frame for a fixed 5-char
# query, peak/steady RSS/PSS while open, and 30 s idle cost. Writes raw
# per-iteration milliseconds plus machine state; stats.awk beside this
# file turns a column into timeouts/n/min/median/p95/max/mean.
#
# Usage (on any aarch64/x86_64 Linux box with a Wayland-capable scoot):
#   BIN_DIR=~/w36-ab OUT=~/w36-ab/results N=30 scripts/launcher-ab-bench/bench.sh
#
# Contract:
#   $BIN_DIR/scoot and $BIN_DIR/scootlaunch are the RELEASED binaries
#     under test (on NixOS without nix-ld, patched copies whose loader
#     and rpath point at nixpkgs: the bytes are the release, only the
#     interpreter path differs; the results file records sha256sums).
#   fuzzel comes from PATH (on the M2: nixpkgs fuzzel, version recorded).
#   DATA (default $OUT/data) holds the synthetic entry dirs made by
#     gen-entries.sh beside this file (100/1000/5000); SIZES overrides.
#   FONT (default: fc-match "DejaVu Sans") is passed to scootlaunch
#     --font; fuzzel reads the same family through fontconfig.
# Each launcher runs with XDG_DATA_HOME set to an empty dir and
# XDG_DATA_DIRS set to ONLY the synthetic dir, so the entry count is
# exact. Detection is screenshot-diff polling via `scoot msg
# screenshot`: `windows` lists no layer-shell surfaces, so no cheaper
# poll exists; the script first records the screenshot round-trip
# distribution as the method's timing floor. Nothing is discarded:
# iterations that never show a frame write TIMEOUT and stats.awk counts
# them. The compositor and every launcher are killed by exact PID
# (captured $!), never by pattern. No python is used on the target.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd || exit 1)
STATS_AWK=$SCRIPT_DIR/stats.awk
BIN_DIR=${BIN_DIR:-$PWD}
OUT=${OUT:-$PWD/bench-results}
N=${N:-30}
SIZES=${SIZES:-"100 1000 5000"}
DATA=${DATA:-$OUT/data}
FONT=${FONT:-$(fc-match 'DejaVu Sans' --format='%{file}\n' 2>/dev/null || true)}
FUZZEL=${FUZZEL:-fuzzel}

SCOOT=$BIN_DIR/scoot
SCOOTLAUNCH=$BIN_DIR/scootlaunch
RTDIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export XDG_RUNTIME_DIR="$RTDIR"

mkdir -p "$OUT"
cd "$OUT" || exit 1

{
    echo "--- machine ---"
    uname -a
    echo "load: $(cat /proc/loadavg)"
    echo "nproc: $(nproc)"
    echo "--- binaries ---"
    sha256sum "$SCOOT" "$SCOOTLAUNCH"
    ls -l "$SCOOT" "$SCOOTLAUNCH"
    "$SCOOT" --version
    "$SCOOTLAUNCH" --version
    command -v "$FUZZEL"
    "$FUZZEL" --version
    echo "--- font ---"
    echo "FONT=$FONT"
} >meta.txt 2>&1
cat meta.txt

if [ ! -x "$SCOOT" ] || [ ! -x "$SCOOTLAUNCH" ]; then
    echo "bench.sh: BIN_DIR=$BIN_DIR needs executable scoot and scootlaunch" >&2
    exit 2
fi
if ! command -v "$FUZZEL" >/dev/null 2>&1; then
    echo "bench.sh: no fuzzel on PATH (FUZZEL=$FUZZEL)" >&2
    exit 2
fi
if [ -z "$FONT" ]; then
    echo "bench.sh: no font found (set FONT=path/to/DejaVuSans.ttf)" >&2
    exit 2
fi
# shellcheck disable=SC2086 # SIZES is a word list
for size in $SIZES; do
    if [ ! -d "$DATA/$size/applications" ]; then
        echo "bench.sh: missing $DATA/$size/applications (run gen-entries.sh)" >&2
        exit 2
    fi
done

# --- compositor (exact-PID lifecycle) ---
SOCK=$OUT/scoot-bench.sock
SCOOT_SOCKET=$SOCK
export SCOOT_SOCKET
rm -f "$SOCK" comp.log
"$SCOOT" --headless --width 800 --height 600 --socket "$SOCK" \
    --config /dev/null >comp.log 2>&1 &
COMPOSITOR=$!
cleanup() {
    if kill -0 "$COMPOSITOR" 2>/dev/null; then kill "$COMPOSITOR" 2>/dev/null || true; fi
    for _ in $(seq 1 50); do
        kill -0 "$COMPOSITOR" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$COMPOSITOR" 2>/dev/null; then kill -9 "$COMPOSITOR" 2>/dev/null || true; fi
}
trap cleanup EXIT
for _ in $(seq 1 100); do
    [ -S "$SOCK" ] && break
    sleep 0.1
done
if [ ! -S "$SOCK" ]; then
    echo "bench.sh: control socket never appeared; compositor log:" >&2
    tail -20 comp.log >&2
    exit 1
fi
WAYLAND_DISPLAY=$(grep -o 'wayland="[a-z0-9-]*"' comp.log | head -1 | cut -d'"' -f2)
if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "bench.sh: no wayland socket in comp.log" >&2
    tail -5 comp.log >&2
    exit 1
fi
export WAYLAND_DISPLAY
echo "compositor pid=$COMPOSITOR wayland=$WAYLAND_DISPLAY"
"$SCOOT" msg version

# --- timing floor: bare screenshot round trips, nothing open ---
"$SCOOT" msg screenshot --out base.png --no-cursor >/dev/null
: >floor.txt
i=0
while [ "$i" -lt "$N" ]; do
    s=$(date +%s%N)
    "$SCOOT" msg screenshot --out floor.png --no-cursor >/dev/null
    e=$(date +%s%N)
    echo $(( (e - s) / 1000000 )) >>floor.txt
    i=$((i + 1))
done
echo "floor (screenshot RTT ms): $(awk -f "$STATS_AWK" floor.txt)"

shot_differs() {
    local out
    # shot_differs OUTFILE: one screenshot; true (rc 0) when it differs
    # from the launcher-less base frame.
    "$SCOOT" msg screenshot --out "$1" --no-cursor >/dev/null 2>&1
    ! cmp -s base.png "$1"
}

wait_frame() {
    local out maxpolls p
    # wait_frame OUTFILE MAXPOLLS: poll until the frame differs from
    # base; prints polls used, rc 0 on success.
    out=$1
    maxpolls=$2
    p=0
    while [ "$p" -lt "$maxpolls" ]; do
        if shot_differs "$out"; then
            echo "$p"
            return 0
        fi
        p=$((p + 1))
    done
    echo "$p"
    return 1
}

launch_cell() {
    local launcher size state home s e pid
    # launch_cell LAUNCHER SIZE STATE: one first-frame measurement in ms
    # (or TIMEOUT). STATE is cold|warm for scootlaunch (fresh vs kept
    # index cache), ignored for fuzzel which parses every launch.
    launcher=$1
    size=$2
    state=$3
    home=$OUT/home-empty
    mkdir -p "$home"
    if [ "$launcher" = "scootlaunch" ] && [ "$state" = "cold" ]; then
        rm -rf "$OUT/cache-cold"
        mkdir -p "$OUT/cache-cold" "$OUT/state-cold"
        export XDG_CACHE_HOME=$OUT/cache-cold XDG_STATE_HOME=$OUT/state-cold
    else
        mkdir -p "$OUT/cache-warm" "$OUT/state-warm"
        export XDG_CACHE_HOME=$OUT/cache-warm XDG_STATE_HOME=$OUT/state-warm
    fi
    export XDG_DATA_HOME=$home XDG_DATA_DIRS=$DATA/$size
    s=$(date +%s%N)
    if [ "$launcher" = "scootlaunch" ]; then
        "$SCOOTLAUNCH" --font "$FONT" </dev/null >"$OUT/last-launch.log" 2>&1 &
    else
        "$FUZZEL" </dev/null >"$OUT/last-launch.log" 2>&1 &
    fi
    pid=$!
    if wait_frame "$OUT/shot.png" 200 >/dev/null; then
        e=$(date +%s%N)
        echo $(( (e - s) / 1000000 ))
    else
        echo "TIMEOUT"
    fi
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
    for _ in $(seq 1 50); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
    wait "$pid" 2>/dev/null || true
}

query_cell() {
    local launcher size state query home s k e pid p found
    # query_cell LAUNCHER SIZE STATE QUERY: total launch+query ms and the
    # post-keystroke update ms, tab-separated (or TIMEOUT twice).
    # scootlaunch STATE cold uses a fresh cache (no index); warm reuses
    # the pre-warmed one. fuzzel parses every launch: STATE is recorded
    # but changes nothing.
    launcher=$1
    size=$2
    state=$3
    query=$4
    home=$OUT/home-empty
    mkdir -p "$home"
    if [ "$launcher" = "scootlaunch" ] && [ "$state" = "cold" ]; then
        rm -rf "$OUT/qcache-cold"
        mkdir -p "$OUT/qcache-cold" "$OUT/qstate-cold"
        export XDG_CACHE_HOME=$OUT/qcache-cold XDG_STATE_HOME=$OUT/qstate-cold
    else
        mkdir -p "$OUT/qcache-warm" "$OUT/qstate-warm"
        export XDG_CACHE_HOME=$OUT/qcache-warm XDG_STATE_HOME=$OUT/qstate-warm
    fi
    export XDG_DATA_HOME=$home XDG_DATA_DIRS=$DATA/$size
    s=$(date +%s%N)
    if [ "$launcher" = "scootlaunch" ]; then
        "$SCOOTLAUNCH" --font "$FONT" </dev/null >"$OUT/last-query.log" 2>&1 &
    else
        "$FUZZEL" </dev/null >"$OUT/last-query.log" 2>&1 &
    fi
    pid=$!
    if ! wait_frame "$OUT/qref.png" 200 >/dev/null; then
        if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
        wait "$pid" 2>/dev/null || true
        printf 'TIMEOUT\tTIMEOUT\n'
        return 0
    fi
    sleep 0.5
    # Settle, then the pre-keystroke reference: whatever the frame holds
    # here (caret phase included) predates the query, so the first
    # difference after typing is the update. Caret blink between the
    # reference and the keystroke is the residual race; see the doc.
    "$SCOOT" msg screenshot --out "$OUT/qref.png" --no-cursor >/dev/null 2>&1
    k=$(date +%s%N)
    "$SCOOT" msg type "$query" >/dev/null 2>&1 || true
    p=0
    found=0
    while [ "$p" -lt 200 ]; do
        "$SCOOT" msg screenshot --out "$OUT/qshot.png" --no-cursor >/dev/null 2>&1
        if ! cmp -s "$OUT/qref.png" "$OUT/qshot.png"; then
            found=1
            break
        fi
        p=$((p + 1))
    done
    e=$(date +%s%N)
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
    for _ in $(seq 1 50); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
    wait "$pid" 2>/dev/null || true
    if [ "$found" = "1" ]; then
        printf '%d\t%d\n' "$(( (e - s) / 1000000 ))" "$(( (e - k) / 1000000 ))"
    else
        printf 'TIMEOUT\tTIMEOUT\n'
    fi
}

mem_cell() {
    local launcher size home pid j rss pss peak_rss peak_pss steady_rss steady_pss
    # mem_cell LAUNCHER SIZE: peak and steady Rss/Pss (kB) while open.
    launcher=$1
    size=$2
    home=$OUT/home-empty
    mkdir -p "$home" "$OUT/cache-warm" "$OUT/state-warm"
    export XDG_DATA_HOME=$home XDG_DATA_DIRS=$DATA/$size
    export XDG_CACHE_HOME=$OUT/cache-warm XDG_STATE_HOME=$OUT/state-warm
    if [ "$launcher" = "scootlaunch" ]; then
        "$SCOOTLAUNCH" --font "$FONT" </dev/null >"$OUT/last-mem.log" 2>&1 &
    else
        "$FUZZEL" </dev/null >"$OUT/last-mem.log" 2>&1 &
    fi
    pid=$!
    wait_frame "$OUT/mem.png" 200 >/dev/null || true
    sleep 0.5
    peak_rss=0
    peak_pss=0
    rss=0
    pss=0
    j=0
    while [ "$j" -lt 20 ]; do
        if [ -r "/proc/$pid/smaps_rollup" ]; then
            rss=$(awk '/^Rss:/ {print $2}' "/proc/$pid/smaps_rollup")
            pss=$(awk '/^Pss:/ {print $2}' "/proc/$pid/smaps_rollup")
            [ "${rss:-0}" -gt "$peak_rss" ] && peak_rss=$rss
            [ "${pss:-0}" -gt "$peak_pss" ] && peak_pss=$pss
        fi
        sleep 0.1
        j=$((j + 1))
    done
    steady_rss=$rss
    steady_pss=$pss
    echo "peak_rss_kb=$peak_rss peak_pss_kb=$peak_pss steady_rss_kb=$steady_rss steady_pss_kb=$steady_pss"
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
    for _ in $(seq 1 50); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
    wait "$pid" 2>/dev/null || true
}

idle_cell() {
    local launcher size home pid ut0 st0 v0 ut1 st1 v1
    # idle_cell LAUNCHER SIZE: utime+stime ticks and voluntary context
    # switches over 30 s with the launcher open and idle.
    launcher=$1
    size=$2
    home=$OUT/home-empty
    mkdir -p "$home" "$OUT/cache-warm" "$OUT/state-warm"
    export XDG_DATA_HOME=$home XDG_DATA_DIRS=$DATA/$size
    export XDG_CACHE_HOME=$OUT/cache-warm XDG_STATE_HOME=$OUT/state-warm
    if [ "$launcher" = "scootlaunch" ]; then
        "$SCOOTLAUNCH" --font "$FONT" </dev/null >"$OUT/last-idle.log" 2>&1 &
    else
        "$FUZZEL" </dev/null >"$OUT/last-idle.log" 2>&1 &
    fi
    pid=$!
    wait_frame "$OUT/idle.png" 200 >/dev/null || true
    sleep 1
    if ! kill -0 "$pid" 2>/dev/null; then
        echo "launcher died during idle wait"
        wait "$pid" 2>/dev/null || true
        return 0
    fi
    read -r ut0 st0 < <(awk '{print $14, $15}' "/proc/$pid/stat")
    v0=$(awk '/^voluntary_ctxt_switches:/ {print $2}' "/proc/$pid/status")
    sleep 30
    if ! kill -0 "$pid" 2>/dev/null; then
        echo "launcher died during idle sleep"
        wait "$pid" 2>/dev/null || true
        return 0
    fi
    read -r ut1 st1 < <(awk '{print $14, $15}' "/proc/$pid/stat")
    v1=$(awk '/^voluntary_ctxt_switches:/ {print $2}' "/proc/$pid/status")
    : "${ut0:=0}" "${st0:=0}" "${ut1:=0}" "${st1:=0}" "${v0:=0}" "${v1:=0}"
    echo "cpu_ticks=$(( ut1 + st1 - ut0 - st0 )) voluntary_ctxt=$(( v1 - v0 ))"
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
    for _ in $(seq 1 50); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
    wait "$pid" 2>/dev/null || true
}

# Pre-warm the shared scootlaunch caches so warm cells measure indexed.
export XDG_DATA_HOME=$OUT/home-empty XDG_DATA_DIRS=$DATA/100
mkdir -p "$OUT/home-empty" "$OUT/cache-warm" "$OUT/state-warm"
export XDG_CACHE_HOME=$OUT/cache-warm XDG_STATE_HOME=$OUT/state-warm
"$SCOOTLAUNCH" --list >/dev/null 2>&1 || true
mkdir -p "$OUT/qcache-warm" "$OUT/qstate-warm"
XDG_CACHE_HOME=$OUT/qcache-warm XDG_STATE_HOME=$OUT/qstate-warm \
    "$SCOOTLAUNCH" --list >/dev/null 2>&1 || true

echo "=== M1 launch-to-first-frame (N=$N) ==="
# shellcheck disable=SC2086 # SIZES is a word list
for size in $SIZES; do
    for cell in "scootlaunch cold" "scootlaunch warm" "fuzzel warm"; do
        # shellcheck disable=SC2086
        set -- $cell
        launcher=$1
        state=$2
        out=launch-${launcher}-${size}-${state}.txt
        : >"$out"
        i=0
        while [ "$i" -lt "$N" ]; do
            launch_cell "$launcher" "$size" "$state" >>"$out"
            i=$((i + 1))
        done
        echo "M1 $launcher size=$size state=$state: $(awk -f "$STATS_AWK" "$out")"
    done
done

echo "=== M2 launch+query (N=$N, query=term5) ==="
# shellcheck disable=SC2086 # SIZES is a word list
for size in $SIZES; do
    q="term5"
    for cell in "scootlaunch cold" "scootlaunch warm" "fuzzel warm"; do
        # shellcheck disable=SC2086
        set -- $cell
        launcher=$1
        state=$2
        out=query-${launcher}-${size}-${state}.txt
        : >"$out"
        i=0
        while [ "$i" -lt "$N" ]; do
            # query_cell prints TOTAL<TAB>UPDATE; keep both columns.
            query_cell "$launcher" "$size" "$state" "$q" >>"$out"
            i=$((i + 1))
        done
        echo "M2 $launcher size=$size state=$state query=$q total: $(cut -f1 "$out" | awk -f "$STATS_AWK")"
        echo "M2 $launcher size=$size state=$state query=$q update: $(cut -f2 "$out" | awk -f "$STATS_AWK")"
    done
done

echo "=== M3 memory ==="
# shellcheck disable=SC2086 # SIZES is a word list
for size in $SIZES; do
    for launcher in scootlaunch fuzzel; do
        echo -n "M3 $launcher size=$size: "
        mem_cell "$launcher" "$size" | tee "mem-${launcher}-${size}.txt"
    done
done

echo "=== M4 idle 30 s ==="
for launcher in scootlaunch fuzzel; do
    echo -n "M4 $launcher size=100: "
    idle_cell "$launcher" 100 | tee "idle-${launcher}.txt"
done
sl_base=$(basename "$SCOOTLAUNCH")
fz_base=$(basename "$FUZZEL")
echo -n "M4 scootlaunch resident when closed: "
if pgrep -x "$sl_base" >/dev/null 2>&1; then
    echo "PROCESS STILL RUNNING ($sl_base)"
else
    echo "no $sl_base process, no daemon"
fi
echo -n "M4 fuzzel resident when closed: "
if pgrep -x "$fz_base" >/dev/null 2>&1; then
    echo "PROCESS STILL RUNNING ($fz_base)"
else
    echo "no $fz_base process, no daemon"
fi

echo "=== done ==="
ls -la "$OUT"
