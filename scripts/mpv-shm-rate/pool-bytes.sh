#!/usr/bin/env bash
# Reads mpv's wl_shm pool backing files straight from /proc and compares
# their bytes against what scoot screenshots show. If the pool bytes carry
# the dots, the artifact is client-written, not compositor-rendered.
# usage: pool-bytes.sh <pixman|gles> <outdir>
set -u
RENDERER=$1; D=$2
SCOOT=/var/cargo-target/debug/scoot
SCOOTCTL=/var/cargo-target/debug/scootctl
MPV=/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv
CLIP=/home/dev/evidence/gdf/testsrc-nv12.mkv
mkdir -p "$D"
rm -f "$D/scoot.sock"
RUST_LOG=scoot=warn "$SCOOT" --headless --renderer "$RENDERER" --socket "$D/scoot.sock" \
  -- sh -c "exec $MPV --no-config --vo=gpu --gpu-context=wayland --pause $CLIP > $D/mpv.log 2>&1" \
  > "$D/scoot.log" 2>&1 &
CPID=$!
for t in $(seq 1 50); do [ -S "$D/scoot.sock" ] && break; sleep 0.1; done
sleep 5
MPV_PID=$(pgrep -f "testsrc-nv12.mkv" | head -1)
echo "mpv pid: $MPV_PID"
ls -la /proc/$MPV_PID/fd/ 2>/dev/null | grep -vE 'socket|pipe|null|/dev/(null|urandom)' | head -20
echo "--- memfd/shm candidates ---"
ls -la /proc/$MPV_PID/fd/ 2>/dev/null | grep -iE 'memfd|shm|deleted' | head
echo "--- map sizes ---"
grep -E 'deleted|memfd' /proc/$MPV_PID/maps 2>/dev/null | head
SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" screenshot --out "$D/shot-1.png" > /dev/null 2>&1
echo -n "$RENDERER shot-1: "
magick "$D/shot-1.png" -crop 766x960+20+20 +repage \
  -format 'mean=%[fx:mean] std=%[fx:standard_deviation]\n' info:
echo "$MPV_PID" > "$D/mpv.pid"
echo "CPID=$CPID MPV_PID=$MPV_PID (session left running for pool reads; tear down manually)"
