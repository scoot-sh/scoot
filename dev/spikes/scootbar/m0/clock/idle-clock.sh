#!/bin/bash
# Start the clock, let it settle 5 s, then sample voluntary/involuntary
# context switches and utime+stime over a fixed window.
BIN=$1; WIN=$2; LOG=$3
$BIN run /etc/localtime >"$LOG.out" 2>&1 &
pid=$!
sleep 5
s() { awk '/^voluntary_ctxt_switches/{v=$2} /^nonvoluntary_ctxt_switches/{n=$2} END{printf "%s %s", v, n}' /proc/$pid/status; echo -n " "; awk '{print $14+$15}' /proc/$pid/stat; }
a=$(s); ta=$(date +%s.%N)
sleep "$WIN"
b=$(s); tb=$(date +%s.%N)
echo "pid=$pid window_s=$(echo "$tb - $ta" | bc) threads=$(ls /proc/$pid/task | wc -l) start(vol nonvol ticks)=$a end=$b rss_kb=$(awk '/VmRSS/{print $2}' /proc/$pid/status) pss_kb=$(awk '/^Pss:/{print $2}' /proc/$pid/smaps_rollup)" >"$LOG"
kill $pid
