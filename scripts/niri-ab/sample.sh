#!/usr/bin/env bash
# One measurement window on an already-running compositor, for the real-GPU
# `--tty` half of the scoot/niri A/B (Asahi.md Test 9), where the benchmark
# cannot launch the compositor itself. Counts the same things, the same way,
# as scripts/niri-ab-bench.sh:
#
#   scripts/niri-ab/sample.sh PID|session SECS [LABEL] [-- COMMAND...]
#
# `session` means the compositor this is running inside: the nearest
# ancestor process named `scoot` or `niri`, or else the niri that
# `$NIRI_SOCKET` names. Use it when running from a terminal in the session
# under test. On a machine whose own desktop is
# scoot, `pgrep -x scoot` would find two, and the wrong one is the live
# desktop.
#
# Samples the compositor's threads, runs COMMAND for at most SECS if one is
# given (the damage for this window) or sleeps SECS, samples again, and
# prints one TSV line:
#   label  comm  wall_s  proc_cpu_ms  cpu_ns  wakeups  threads  rss_kb  pss_kb
# proc_cpu_ms is the process total (/proc/PID/stat utime+stime, 10 ms
# ticks), and it is the number to compare: it keeps the time of threads that
# started and exited inside the window. cpu_ns and wakeups sum only the
# threads alive at both ends (schedstat), so they miss those. niri encodes
# every screenshot on such a thread, and on the dev VM cpu_ns missed 19-23%
# of niri's screenshot CPU.
set -euo pipefail
PID=${1:?usage: sample.sh PID|session SECS [LABEL] [-- COMMAND...]}
SECS=${2:?usage: sample.sh PID|session SECS [LABEL] [-- COMMAND...]}
shift 2
LABEL=sample
if [ $# -gt 0 ] && [ "$1" != "--" ]; then LABEL=$1; shift; fi
if [ "${1:-}" = "--" ]; then shift; fi
if [ "$PID" = session ]; then
    p=$$
    while [ "$p" -gt 1 ]; do
        # Fields after the last ')': a comm may itself contain spaces.
        p=$(sed 's/.*) //' "/proc/$p/stat" | awk '{print $2}')
        case $(cat "/proc/$p/comm" 2>/dev/null) in scoot | niri) PID=$p; break ;; esac
    done
    # niri double-forks what it spawns, so nothing it started has it as an
    # ancestor. Its socket name carries its pid instead:
    # niri.<wayland display>.<pid>.sock, exported to everything it spawns.
    if [ "$PID" = session ] && [ -n "${NIRI_SOCKET:-}" ]; then
        p=${NIRI_SOCKET%.sock}; p=${p##*.}
        case $p in '' | *[!0-9]*) ;; *) [ "$(cat "/proc/$p/comm" 2>/dev/null)" = niri ] && PID=$p ;; esac
    fi
    [ "$PID" != session ] || { echo "sample.sh: not running inside scoot or niri" >&2; exit 1; }
fi
[ -r "/proc/$PID/stat" ] || { echo "sample.sh: no process $PID" >&2; exit 1; }

sums() {
    awk 'FILENAME ~ /schedstat$/ { cpu += $1; runs += $3; n++ }
         END { printf "%d %d %d\n", cpu, runs, n }' /proc/"$PID"/task/*/schedstat 2>/dev/null
}
jiffies() { sed 's/.*) //' "/proc/$PID/stat" | awk '{print $12 + $13}'; }

read -r c0 r0 n0 <<<"$(sums)"; j0=$(jiffies); t0=$(date +%s%N)
if [ $# -gt 0 ]; then timeout "$SECS" "$@" >/dev/null 2>&1 || true; else sleep "$SECS"; fi
read -r c1 r1 n1 <<<"$(sums)"; j1=$(jiffies); t1=$(date +%s%N)
mem=$(awk '/^Rss:/{r=$2} /^Pss:/{p=$2} END{print r"\t"p}' "/proc/$PID/smaps_rollup")
tick=$(getconf CLK_TCK)
printf '%s\t%s\t%.3f\t%d\t%d\t%d\t%s\t%s\n' "$LABEL" "$(cat "/proc/$PID/comm")" "$(awk -v a="$t0" -v b="$t1" 'BEGIN{print (b-a)/1e9}')" \
    $(((j1 - j0) * 1000 / tick)) $((c1 - c0)) $((r1 - r0)) "$n0/$n1" "$mem"
