#!/bin/bash
# W9 launch-prototype measurement harness (dev VM only). Every row prints
# one RESULT line; the full log is the transcript. PIDs are tracked in
# files and killed by PID only -- never a broad pkill (another agent shares
# this box). The compositor binary runs as `w9lc` with a neutral socket
# name so nobody else's cleanup matches it.
set -u
RT=${RT:?set RT to the session runtime dir}
SCOOT=${SCOOT:-/dev/shm/w9lc}
PROTO=${PROTO:-/dev/shm/w9-proto-target/release/launch-proto}
FONT=${FONT:-/nix/store/33wfvwgr4vy0gaznf3wafla7mfni1znw-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf}
export XDG_RUNTIME_DIR=$RT SCOOT_SOCKET=$RT/ipc.sock
PQL() { perl /tmp/w9-probe.pl "$@"; }

scoot_alive() { kill -0 "$(cat $RT/scoot.pid)" 2>/dev/null; }

shot() { # shot NAME -> path on stdout
  $SCOOT msg screenshot --output 1 --out "$RT/$1.png" >/dev/null 2>&1 && echo "$RT/$1.png"
}

# Fast poll: whole-file md5 of a fresh screenshot. Any pixel change flips
# the hash; no PNG decode. ~20 ms per iteration (11 ms shot + md5).
shot_md5() { # NAME -> md5
  local f; f=$(shot "$1") || return 1
  local h; h=$(md5sum "$f" | cut -d' ' -f1)
  rm -f "$f"; echo "$h"
}

# wait_hash_change OLDMD5 TIMEOUT_S T0_NS -> ms at first change, or TIMEOUT
wait_hash_change() {
  local old=$1; local to=$2; local t0=$3
  local end=$(( $(date +%s) + to ))
  while [ "$(date +%s)" -lt "$end" ]; do
    local md; md=$(shot_md5 "w$RANDOM") || continue
    if [ -n "$md" ] && [ "$md" != "$old" ]; then echo $(( ($(date +%s%N) - t0) / 1000000 )); return 0; fi
  done
  echo TIMEOUT; return 1
}

# wait_nonbg X Y W H TIMEOUT_S T0_NS MIN -> ms at first match, or TIMEOUT
wait_nonbg() {
  local x=$1; local y=$2; local w=$3; local h=$4; local to=$5; local t0=$6; local min=${7:-100000}
  local end=$(( $(date +%s) + to ))
  while [ "$(date +%s)" -lt "$end" ]; do
    local f; f=$(shot "w$RANDOM") || continue
    local nb; nb=$(PQL "$f" "$x" "$y" "$w" "$h" | sed 's/.*nonbg=\([0-9]*\).*/\1/')
    rm -f "$f"
    if [ "${nb:-0}" -gt "$min" ]; then echo $(( ($(date +%s%N) - t0) / 1000000 )); return 0; fi
  done
  echo TIMEOUT; return 1
}

# wait_md5_change OLDMD5 X Y W H TIMEOUT_S T0_NS -> ms, or TIMEOUT
wait_md5_change() {
  local old=$1; local x=$2; local y=$3; local w=$4; local h=$5; local to=$6; local t0=$7
  local end=$(( $(date +%s) + to ))
  while [ "$(date +%s)" -lt "$end" ]; do
    local f; f=$(shot "w$RANDOM") || continue
    local md; md=$(PQL "$f" "$x" "$y" "$w" "$h" | sed 's/.*md5=//')
    rm -f "$f"
    if [ -n "$md" ] && [ "$md" != "$old" ]; then echo $(( ($(date +%s%N) - t0) / 1000000 )); return 0; fi
  done
  echo TIMEOUT; return 1
}

start_panel() { # ARGS... -> pid file $RT/proto.pid (the REAL pid: nohup, no setsid)
  rm -f $RT/trace.log
  nohup $PROTO --font "$FONT" --trace $RT/trace.log "$@" >/tmp/w9A-panel-out.txt 2>&1 < /dev/null &
  echo $! > $RT/proto.pid
}

kill_panel() { local p; p=$(cat $RT/proto.pid 2>/dev/null) || return 0; kill "$p" 2>/dev/null; sleep 0.3; kill -9 "$p" 2>/dev/null; return 0; }

# Panel geometry on 1280x800: x 320..960, query strip y 0..36.
PX=320; PW=640; PH=340
