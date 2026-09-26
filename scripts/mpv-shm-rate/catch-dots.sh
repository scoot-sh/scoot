#!/usr/bin/env bash
# Catches a "dots" paused run, then dumps mpv's pool bytes and renders them
# as an image for direct comparison with the screenshot.
# usage: catch-dots.sh <pixman|gles> <outdir> [max-tries]
set -u
RENDERER=$1; D=$2; MAXTRIES=${3:-8}
SCOOT=/var/cargo-target/debug/scoot
SCOOTCTL=/var/cargo-target/debug/scootctl
MPV=/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv
CLIP=/home/dev/evidence/gdf/testsrc-nv12.mkv
mkdir -p "$D"
# MAXTRIES counts scored tries only: a try where the client died early
# (notably WAYLAND_DEBUG=1 + --pause intermittently crashes mpv in
# vo_x11_init, leaving zero windows) is torn down and retried WITHOUT
# scoring — the empty background would otherwise score std<0.01 "black"
# and burn a try it could never win. Bounded so a permanently-crashing
# client still terminates.
scored=0; attempt=0
while [ "$scored" -lt "$MAXTRIES" ]; do
  attempt=$((attempt+1))
  if [ "$attempt" -gt $((MAXTRIES*3)) ]; then
    echo "too many unscorable tries ($attempt attempts, $scored scored); giving up"
    exit 1
  fi
  try=$attempt
  T=$D/try-$try; mkdir -p "$T"
  rm -f "$T/scoot.sock"
  RUST_LOG=scoot=warn "$SCOOT" --headless --renderer "$RENDERER" --socket "$T/scoot.sock" \
    -- sh -c "echo \$\$ > $T/clientpid; exec env WAYLAND_DEBUG=1 $MPV --no-config --vo=gpu --gpu-context=wayland --pause $CLIP > $T/mpv.log 2>&1" \
    > "$T/scoot.log" 2>&1 &
  CPID=$!
  for t in $(seq 1 50); do [ -S "$T/scoot.sock" ] && break; sleep 0.1; done
  sleep 5
  if ! kill -0 "$(cat "$T/clientpid" 2>/dev/null)" 2>/dev/null; then
    echo "try-$try: client exited early, retrying without scoring"
    pkill -f "testsrc-nv12.mkv" 2>/dev/null
    sleep 0.3
    kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
    continue
  fi
  SCOOT_SOCKET="$T/scoot.sock" "$SCOOTCTL" windows > "$T/windows.json" 2>&1
  if ! grep -q '"width"' "$T/windows.json" 2>/dev/null; then
    echo "try-$try: zero windows (client crashed?), retrying without scoring"
    pkill -f "testsrc-nv12.mkv" 2>/dev/null
    sleep 0.3
    kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
    continue
  fi
  SCOOT_SOCKET="$T/scoot.sock" "$SCOOTCTL" screenshot --out "$T/shot.png" > /dev/null 2>&1
  stats=$(magick "$T/shot.png" -crop 766x960+20+20 +repage \
    -format '%[fx:mean],%[fx:standard_deviation],%[fx:maxima]' info: 2>/dev/null)
  echo "try-$try: $stats"
  std=$(echo "$stats" | cut -d, -f2)
  cls=$(awk -v s="$std" 'BEGIN{print (s<0.01)?"black":((s<0.30)?"dots":"solid")}')
  if [ "$cls" = "dots" ]; then
    echo "CAUGHT dots in try-$try"
    MPV_PID=$(cat "$T/clientpid")
    echo "mpv pid: $MPV_PID"
    for fd in $(ls /proc/$MPV_PID/fd/ | head -40); do
      target=$(readlink /proc/$MPV_PID/fd/$fd 2>/dev/null)
      case "$target" in *mesa-shared*) cp /proc/$MPV_PID/fd/$fd "$T/pool-$fd.bin" 2>/dev/null;; esac
    done
    ls -la "$T/"
    for pool in "$T"/pool-*.bin; do
      base=$(basename "$pool" .bin)
      magick -size 782x976 -depth 8 "BGRA:$pool" "$T/$base.png" 2>/dev/null && echo "rendered $base.png"
    done
    echo "buffers: $(grep -c 'create_buffer(' "$T/mpv.log" 2>/dev/null) shm-pools: $(grep -c 'create_pool(' "$T/mpv.log" 2>/dev/null) dma-add: $(grep -cE 'params_v1#[0-9]+\.add\(' "$T/mpv.log" 2>/dev/null) immed: $(grep -c 'create_immed(' "$T/mpv.log" 2>/dev/null)"
    echo "$MPV_PID $CPID" > "$T/pids"
    echo "session left running (try-$try); tear down manually"
    exit 0
  fi
  pkill -f "testsrc-nv12.mkv" 2>/dev/null
  sleep 0.3
  kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
  scored=$((scored+1))
done
echo "no dots in $scored scored tries ($attempt attempts)"
exit 1
