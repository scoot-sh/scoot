#!/usr/bin/env bash
# A/B bench: scootnotify (the release binary) vs mako, each on its own
# private scoot --headless session and private session bus. Measures
# notify-to-first-frame (warm daemon, and the first card of a freshly
# started daemon), resident memory (Rss/Pss/USS) at idle and while a
# card shows, and idle wakeups (voluntary context switches over
# IDLE_SECS-long windows, summed over every thread). Raw per-iteration
# values land in $OUT; stats.awk (shared with the launcher bench) turns a
# column into timeouts/n/min/median/p95/max/mean.
#
# Usage (any aarch64/x86_64 Linux box with busctl and dbus-run-session):
#   BIN_DIR=~/nab/bin MAKO=/path/to/mako OUT=~/nab/results \
#     scripts/notify-ab-bench/bench.sh
# Smoke (about a minute, proves the harness, not a result):
#   N=2 IDLE_N=1 IDLE_SECS=3 BIN_DIR=... MAKO=... scripts/notify-ab-bench/bench.sh
#
# Contract:
#   $BIN_DIR/scoot and $BIN_DIR/scootnotify are the RELEASED binaries
#     under test (on NixOS without nix-ld, patched copies whose loader and
#     rpath point at nixpkgs: same bytes, only the interpreter path
#     differs; meta.txt records sha256sums).
#   MAKO is the mako binary (default: mako on PATH).
#   FONT (default: fc-match "DejaVu Sans") is scootnotify's --font file;
#     mako gets MAKO_FONT ("DejaVu Sans 10", pango points: 13.3 px at
#     96 DPI against scootnotify's 13.0 px default): same face, same
#     nominal size, neither reading operator config. Both cards are
#     anchored top-right, 360 px wide, 12 px margin.
#   N (30) trials per latency cell; IDLE_N (30) windows of IDLE_SECS (60)
#     per idle cell; IDLE_STATES (default "fresh closed"): fresh = a
#     daemon that has never shown a card, closed = after one card was
#     shown and closed (a bar-less desktop's steady state, where
#     scootnotify holds its bar-feed send owed). ORDER ("scootnotify
#     mako") is the order of the latency phases.
#
# Memory is sampled only in the latency phases, where one daemon runs at
# a time: the idle phases run in parallel, so two instances of the same
# binary share its pages and would skew Pss/USS (they did, in a first
# draft: scootnotify USS 1768 kB alone, 224 kB beside its twin).
#
# Isolation: every phase runs under its own dbus-run-session (the
# operator's bus is never touched), its own short XDG_RUNTIME_DIR
# (RT_BASE, default /dev/shm/nab; the Wayland socket, the control
# socket and the screenshots all live there), and an empty
# XDG_CONFIG_HOME/XDG_STATE_HOME/XDG_CACHE_HOME apart from mako's own
# config. The idle phases run in parallel, one stack each, since an idle
# daemon does not contend; the latency phases run alone, one at a time.
# Every process is killed by its captured PID, never by pattern. No
# python is used on the target.
#
# Method: a screenshot-diff poll (`scoot msg screenshot` against the
# card-less base frame; `windows` lists no layer-shell surfaces) is the
# only detector, so its round trip IS the poll period: the floor cells
# measure it, a bare busctl call, and the two together (the part of every
# latency below that is the harness). "First frame" = the first frame
# that differs from base at all (a partly faded-in frame counts).
set -euo pipefail

SELF=$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0")
SCRIPT_DIR=$(dirname "$SELF")
STATS_AWK=$SCRIPT_DIR/../launcher-ab-bench/stats.awk
BIN_DIR=${BIN_DIR:-$PWD}
case "$BIN_DIR" in /*) ;; *) BIN_DIR=$PWD/$BIN_DIR ;; esac
OUT=${OUT:-$PWD/notify-bench-results}
case "$OUT" in /*) ;; *) OUT=$PWD/$OUT ;; esac
N=${N:-30}
IDLE_N=${IDLE_N:-30}
IDLE_SECS=${IDLE_SECS:-60}
IDLE_STATES=${IDLE_STATES:-"fresh closed"}
ORDER=${ORDER:-"scootnotify mako"}
MAKO=${MAKO:-$(command -v mako || true)}
FONT=${FONT:-$(fc-match 'DejaVu Sans' --format='%{file}\n' 2>/dev/null || true)}
MAKOCTL=${MAKOCTL:-$(dirname "${MAKO:-/nonexistent/mako}")/makoctl}
MAKO_FONT=${MAKO_FONT:-"DejaVu Sans 10"}
RT_BASE=${RT_BASE:-/dev/shm/nab}
SCOOT=$BIN_DIR/scoot
SCOOTNOTIFY=$BIN_DIR/scootnotify
DEST=org.freedesktop.Notifications
OPATH=/org/freedesktop/Notifications

# --- one trial's pieces (used inside a phase) ---

bus() { busctl --user "$@"; }

# notify: send one Notify (never expires: expire_timeout 0), print the id.
notify() {
    bus call $DEST $OPATH $DEST Notify susssasa{sv}i \
        bench 0 "" "hello summary" "body text here" 0 0 0 | awk '{print $2}'
}

# close_card ID: through each daemon's own control tool. scootnotify
# refuses a CloseNotification from another bus connection than the one
# that sent the Notify (its cross-sender policy), and every busctl call
# is a new connection.
close_card() {
    if [ "$CUR" = scootnotify ]; then
        "$SCOOTNOTIFY" msg dismiss "$1" >/dev/null
    else
        "$MAKOCTL" dismiss -n "$1" >/dev/null
    fi
}

name_owned() {
    bus call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
        NameHasOwner s $DEST 2>/dev/null | grep -q true
}

shot() { "$SCOOT" msg screenshot --out "$RT/shot.png" --no-cursor >/dev/null 2>&1; }
shot_is_base() { shot && cmp -s "$RT/base.png" "$RT/shot.png"; }

# poll_shown MAXPOLLS: poll until a frame differs from base (rc 0).
poll_shown() {
    local p=0
    while [ "$p" -lt "$1" ]; do
        shot_is_base || return 0
        p=$((p + 1))
    done
    return 1
}

# wait_clear: poll until the frame is the base again (exit fades finish).
wait_clear() {
    local p=0
    while [ "$p" -lt 200 ]; do
        shot_is_base && return 0
        p=$((p + 1))
        sleep 0.02
    done
    return 1
}

# mem_sample PID: "rss pss uss" kB from smaps_rollup (uss = Private_*).
mem_sample() {
    awk '/^Rss:/ {r=$2} /^Pss:/ {p=$2} /^Private_(Clean|Dirty):/ {u+=$2}
         END {print r+0, p+0, u+0}' "/proc/$1/smaps_rollup"
}

# vol_sum PID: voluntary context switches over every thread, and the
# thread count: "vol threads".
vol_sum() {
    local pid=$1
    awk 'FNR==1 {t++} /^voluntary_ctxt_switches:/ {v+=$2}
         END {print v+0, t+0}' /proc/"$pid"/task/*/status
}

# cpu_ticks PID: utime+stime of the process (comm has no spaces here).
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }

# --- the phase's private stack ---

stack_up() {
    mkdir -p "$RT_BASE"
    RT=$(mktemp -d "$RT_BASE/$1.XXXXXX")
    chmod 700 "$RT"
    mkdir -p "$RT/cfg/mako" "$RT/state" "$RT/cache"
    printf 'font=%s\nwidth=360\nmargin=12\nanchor=top-right\n' "$MAKO_FONT" \
        >"$RT/cfg/mako/config"
    export XDG_RUNTIME_DIR=$RT XDG_CONFIG_HOME=$RT/cfg XDG_STATE_HOME=$RT/state \
        XDG_CACHE_HOME=$RT/cache SCOOT_SOCKET=$RT/s.sock
    "$SCOOT" --headless --width 800 --height 600 --socket "$SCOOT_SOCKET" \
        --config /dev/null >"$OUT/$1-comp.log" 2>&1 &
    COMP=$!
    local i
    for i in $(seq 1 100); do
        [ -S "$SCOOT_SOCKET" ] && break
        kill -0 "$COMP" 2>/dev/null || break
        sleep 0.1
    done
    if [ ! -S "$SCOOT_SOCKET" ]; then
        echo "bench: compositor never came up:" >&2
        tail -20 "$OUT/$1-comp.log" >&2
        return 1
    fi
    WAYLAND_DISPLAY=$(grep -o 'wayland="[a-z0-9-]*"' "$OUT/$1-comp.log" | head -1 | cut -d'"' -f2)
    if [ -z "$WAYLAND_DISPLAY" ]; then
        echo "bench: no wayland socket in $OUT/$1-comp.log" >&2
        return 1
    fi
    export WAYLAND_DISPLAY
    DPID=""
}

stack_down() {
    daemon_stop 2>/dev/null || true
    if [ -n "${COMP:-}" ] && kill -0 "$COMP" 2>/dev/null; then
        kill "$COMP" 2>/dev/null || true
        for _ in $(seq 1 50); do kill -0 "$COMP" 2>/dev/null || break; sleep 0.1; done
        kill -0 "$COMP" 2>/dev/null && kill -9 "$COMP" 2>/dev/null || true
        wait "$COMP" 2>/dev/null || true
    fi
    if [ -n "${RT:-}" ]; then rm -rf "$RT"; fi
}

# daemon_start NAME LOG: start the daemon, wait until it owns the bus
# name; fail fast (and show its log) if it dies first.
daemon_start() {
    local name=$1 log=$2 i
    if [ "$name" = scootnotify ]; then
        "$SCOOTNOTIFY" daemon --font "$FONT" </dev/null >"$log" 2>&1 &
    else
        "$MAKO" </dev/null >"$log" 2>&1 &
    fi
    DPID=$!
    for i in $(seq 1 500); do
        if ! kill -0 "$DPID" 2>/dev/null; then
            echo "bench: $name died before owning the name; log:" >&2
            cat "$log" >&2
            DPID=""
            return 1
        fi
        name_owned && return 0
        sleep 0.02
    done
    echo "bench: $name never owned $DEST (state: $(grep -E '^State' "/proc/$DPID/status" 2>/dev/null)); log:" >&2
    cat "$log" >&2
    return 1
}

# daemon_stop: SIGTERM by PID, reap, wait for the name to be released
# (a respawn before that exits 3).
daemon_stop() {
    [ -n "${DPID:-}" ] || return 0
    local i
    if kill -0 "$DPID" 2>/dev/null; then kill "$DPID" 2>/dev/null || true; fi
    for i in $(seq 1 100); do kill -0 "$DPID" 2>/dev/null || break; sleep 0.05; done
    kill -0 "$DPID" 2>/dev/null && kill -9 "$DPID" 2>/dev/null || true
    wait "$DPID" 2>/dev/null || true
    DPID=""
    for i in $(seq 1 100); do name_owned || return 0; sleep 0.05; done
    echo "bench: name still owned after the daemon exited" >&2
    return 1
}

# base_frame: record the card-less base and check it is byte-stable.
base_frame() {
    local i
    shot
    cp "$RT/shot.png" "$RT/base.png"
    for i in 1 2 3 4 5; do
        if ! shot_is_base; then
            echo "bench: card-less frame not stable (daemon already drawing?)" >&2
            return 1
        fi
    done
}

# --- phases ---

phase_latency() {
    local name=$1 out i
    CUR=$name
    stack_up "lat-$name"
    trap stack_down EXIT
    out=$OUT/$name
    daemon_start "$name" "$out-daemon.log"
    sleep 1
    base_frame

    # Floors, with the daemon idle and no card: the screenshot round
    # trip (= the poll period), a bare bus round trip, and both.
    : >"$out-floor-shot.txt"
    : >"$out-floor-bus.txt"
    : >"$out-floor-both.txt"
    for i in $(seq 1 "$N"); do
        local s e
        s=$(date +%s%N)
        shot
        e=$(date +%s%N)
        echo $(((e - s) / 1000000)) >>"$out-floor-shot.txt"
        s=$(date +%s%N)
        bus call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId >/dev/null
        e=$(date +%s%N)
        echo $(((e - s) / 1000000)) >>"$out-floor-bus.txt"
        s=$(date +%s%N)
        bus call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId >/dev/null
        shot
        e=$(date +%s%N)
        echo $(((e - s) / 1000000)) >>"$out-floor-both.txt"
    done

    # Warm: one long-lived daemon, three untimed warm-ups, then N timed.
    for i in 1 2 3; do
        local id
        id=$(notify)
        poll_shown 400 || echo "bench: $name warm-up: card never showed" >&2
        close_card "$id"
        wait_clear || echo "bench: $name warm-up: frame never cleared" >&2
    done
    : >"$out-warm.txt"
    for i in $(seq 1 "$N"); do
        local s e id
        s=$(date +%s%N)
        id=$(notify)
        if poll_shown 400; then
            e=$(date +%s%N)
            echo $(((e - s) / 1000000)) >>"$out-warm.txt"
        else
            echo TIMEOUT >>"$out-warm.txt"
        fi
        close_card "$id"
        wait_clear || echo "bench: $name warm trial $i: frame never cleared" >&2
    done
    echo "$(mem_sample "$DPID")" >"$out-mem-after-close.txt"
    daemon_stop

    # Cold: a fresh daemon per trial; idle memory before the card, the
    # first card's latency, memory while it shows.
    : >"$out-cold.txt"
    : >"$out-mem-cold.txt"
    for i in $(seq 1 "$N"); do
        local s e id idle show
        daemon_start "$name" "$out-daemon.log"
        sleep 1
        shot_is_base || { echo "bench: $name cold $i: not clear at start" >&2; }
        idle=$(mem_sample "$DPID")
        s=$(date +%s%N)
        id=$(notify)
        if poll_shown 400; then
            e=$(date +%s%N)
            echo $(((e - s) / 1000000)) >>"$out-cold.txt"
        else
            echo TIMEOUT >>"$out-cold.txt"
        fi
        sleep 0.5
        show=$(mem_sample "$DPID")
        # Still showing after the settle (never-expire held)?
        shot_is_base && echo "bench: $name cold $i: card gone before the memory sample" >&2
        echo "$idle $show" >>"$out-mem-cold.txt"
        close_card "$id"
        daemon_stop
    done
}

phase_idle() {
    local name=$1 state=$2 out w v0 v1 c0 c1 t id
    CUR=$name
    stack_up "idle-$name-$state"
    trap stack_down EXIT
    out=$OUT/idle-$name-$state
    daemon_start "$name" "$out-daemon.log"
    sleep 1
    base_frame
    if [ "$state" = closed ]; then
        id=$(notify)
        poll_shown 400 || echo "bench: $name: the priming card never showed" >&2
        sleep 0.5
        close_card "$id"
        wait_clear || echo "bench: $name: priming card never cleared" >&2
    fi
    sleep 2
    : >"$out.txt"
    read -r v0 t <<<"$(vol_sum "$DPID")"
    c0=$(cpu_ticks "$DPID")
    for w in $(seq 1 "$IDLE_N"); do
        sleep "$IDLE_SECS"
        if ! kill -0 "$DPID" 2>/dev/null; then
            echo "bench: $name died during idle window $w" >&2
            return 1
        fi
        read -r v1 t <<<"$(vol_sum "$DPID")"
        c1=$(cpu_ticks "$DPID")
        echo "$((v1 - v0)) $((c1 - c0)) $t" >>"$out.txt"
        v0=$v1
        c0=$c1
    done
}

# --- orchestrator ---

if [ "${1:-}" = "--phase" ]; then
    mkdir -p "$OUT"
    shift
    kind=$1
    shift
    "phase_$kind" "$@"
    exit $?
fi

if [ ! -x "$SCOOT" ] || [ ! -x "$SCOOTNOTIFY" ]; then
    echo "bench.sh: BIN_DIR=$BIN_DIR needs executable scoot and scootnotify" >&2
    exit 2
fi
if [ -z "$MAKO" ] || [ ! -x "$MAKO" ]; then
    echo "bench.sh: no mako (set MAKO=/path/to/mako)" >&2
    exit 2
fi
if [ ! -x "$MAKOCTL" ]; then
    echo "bench.sh: no makoctl at $MAKOCTL (set MAKOCTL=)" >&2
    exit 2
fi
if [ -z "$FONT" ] || [ ! -r "$FONT" ]; then
    echo "bench.sh: no font found (set FONT=path/to/DejaVuSans.ttf)" >&2
    exit 2
fi
for tool in busctl dbus-run-session; do
    command -v "$tool" >/dev/null 2>&1 || { echo "bench.sh: needs $tool" >&2; exit 2; }
done
mkdir -p "$OUT"
export BIN_DIR OUT N IDLE_N IDLE_SECS MAKO MAKOCTL FONT MAKO_FONT RT_BASE

{
    echo "--- machine ---"
    uname -a
    echo "date: $(date -u +%FT%TZ)"
    echo "load at start: $(cat /proc/loadavg)"
    echo "nproc: $(nproc)"
    echo "--- other compositors/daemons already running (not ours; left alone) ---"
    ps -eo pid,comm,args | awk '$2 ~ /scoot|mako|dbus-daemon/' || true
    echo "--- binaries ---"
    sha256sum "$SCOOT" "$SCOOTNOTIFY" "$MAKO"
    "$SCOOT" --version
    "$SCOOTNOTIFY" --version
    echo "mako: $MAKO (makoctl: $MAKOCTL)"
    echo "--- font ---"
    echo "FONT=$FONT"
    echo "MAKO_FONT=$MAKO_FONT"
    echo "--- parameters ---"
    echo "N=$N IDLE_N=$IDLE_N IDLE_SECS=$IDLE_SECS IDLE_STATES=$IDLE_STATES ORDER=$ORDER"
} >"$OUT/meta.txt" 2>&1
cat "$OUT/meta.txt"

stats() { awk -f "$STATS_AWK" "$1"; }
col() { cut -d' ' -f"$2" "$1" | awk -f "$STATS_AWK"; }

for d in $ORDER; do
    dbus-run-session -- "$SELF" --phase latency "$d"
    echo "=== latency $d ==="
    echo "floor screenshot RTT (ms): $(stats "$OUT/$d-floor-shot.txt")"
    echo "floor bus round trip (ms): $(stats "$OUT/$d-floor-bus.txt")"
    echo "floor bus+screenshot (ms): $(stats "$OUT/$d-floor-both.txt")"
    echo "warm notify-to-first-frame (ms): $(stats "$OUT/$d-warm.txt")"
    echo "cold first card (ms): $(stats "$OUT/$d-cold.txt")"
    echo "cold idle Rss kB: $(col "$OUT/$d-mem-cold.txt" 1)"
    echo "cold idle Pss kB: $(col "$OUT/$d-mem-cold.txt" 2)"
    echo "cold idle USS kB: $(col "$OUT/$d-mem-cold.txt" 3)"
    echo "card-showing Rss kB: $(col "$OUT/$d-mem-cold.txt" 4)"
    echo "card-showing Pss kB: $(col "$OUT/$d-mem-cold.txt" 5)"
    echo "card-showing USS kB: $(col "$OUT/$d-mem-cold.txt" 6)"
    echo "after the warm run (N+3 cards shown and closed) rss/pss/uss kB: $(cat "$OUT/$d-mem-after-close.txt")"
done

if [ "$IDLE_N" -gt 0 ]; then
    pids=""
    for d in $ORDER; do
        for s in $IDLE_STATES; do
            dbus-run-session -- "$SELF" --phase idle "$d" "$s" &
            pids="$pids $!"
        done
    done
    rc=0
    for p in $pids; do wait "$p" || rc=1; done
    for d in $ORDER; do
        for s in $IDLE_STATES; do
            f=$OUT/idle-$d-$s.txt
            echo "=== idle $d $s ($IDLE_N windows of ${IDLE_SECS}s) ==="
            echo "voluntary ctxt switches per window: $(col "$f" 1)"
            echo "cpu ticks per window: $(col "$f" 2)"
            echo "threads: $(col "$f" 3)"
        done
    done
    [ "$rc" = 0 ] || { echo "bench.sh: an idle phase failed" >&2; exit 1; }
fi
{
    echo "load at end: $(cat /proc/loadavg)"
    echo "--- leftovers (ours should be none) ---"
    ps -eo pid,comm,args | awk '$2 == "scootnotify" || $2 == "mako"' || true
} | tee -a "$OUT/meta.txt"
echo "=== done ==="
