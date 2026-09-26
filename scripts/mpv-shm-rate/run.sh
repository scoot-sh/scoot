#!/usr/bin/env bash
# Rate harness for docs/backlog/core/gles-mpv-shm-black.md.
#
# Loops N runs per renderer: start scoot --headless, play the testsrc clip in
# mpv (--vo=gpu, Mesa wl_shm swrast path), take 4 screenshots ~300ms apart,
# tear down. Scoring is scripts/mpv-shm-rate/score.sh (ImageMagick mean/std
# over the window's content rect).
#
# usage: run.sh <pixman|gles> <nruns> <outdir> [scoot-bin] [scootctl-bin]
set -u
RENDERER=$1; N=$2; OUTBASE=$3
SCOOT=${4:-/var/cargo-target/debug/scoot}
SCOOTCTL=${5:-/var/cargo-target/debug/scootctl}
MPV=/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv
CLIP=/home/dev/evidence/gdf/testsrc-nv12.mkv
mkdir -p "$OUTBASE"
echo "build: $($SCOOT --version 2>&1) sha=$({ sha256sum "$SCOOT" || shasum -a 256 "$SCOOT"; } | cut -c1-16) renderer=$RENDERER runs=$N" | tee "$OUTBASE/SETUP"
for i in $(seq 1 "$N"); do
  D=$OUTBASE/run-$i; mkdir -p "$D"
  SOCK=$D/scoot.sock
  rm -f "$SOCK"
  RUST_LOG=scoot=warn "$SCOOT" --headless --renderer "$RENDERER" --socket "$SOCK" \
    -- sh -c "exec $MPV --no-config --vo=gpu --gpu-context=wayland --loop $CLIP > $D/mpv.log 2>&1" \
    > "$D/scoot.log" 2>&1 &
  CPID=$!
  ok=0
  for t in $(seq 1 100); do [ -S "$SOCK" ] && { ok=1; break; }; sleep 0.1; done
  if [ "$ok" = 0 ]; then echo "run-$i: NO SOCKET" | tee -a "$OUTBASE/SETUP"; kill $CPID 2>/dev/null; wait $CPID 2>/dev/null; continue; fi
  sleep 4
  if pgrep -f "testsrc-nv12.mkv" > /dev/null; then echo alive > "$D/client"; else echo exited > "$D/client"; fi
  SCOOT_SOCKET=$SOCK "$SCOOTCTL" windows > "$D/windows.json" 2>&1
  for s in 1 2 3 4; do
    SCOOT_SOCKET=$SOCK "$SCOOTCTL" screenshot --out "$D/shot-$s.png" > /dev/null 2>&1 \
      || echo "run-$i shot-$s FAILED" | tee -a "$OUTBASE/SETUP"
    sleep 0.3
  done
  pkill -f "testsrc-nv12.mkv" 2>/dev/null
  sleep 0.3
  kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
  echo "run-$i done"
done
