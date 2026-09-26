#!/usr/bin/env bash
# One paused-mpv run with selectable vo: isolates client GL rendering from
# compositor handling. usage: paused-vo.sh <pixman|gles> <vo> <outdir>
set -u
RENDERER=$1; VO=$2; D=$3
SCOOT=/var/cargo-target/debug/scoot
SCOOTCTL=/var/cargo-target/debug/scootctl
MPV=/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv
CLIP=/home/dev/evidence/gdf/testsrc-nv12.mkv
mkdir -p "$D"
rm -f "$D/scoot.sock"
RUST_LOG=scoot=warn "$SCOOT" --headless --renderer "$RENDERER" --socket "$D/scoot.sock" \
  -- sh -c "exec env WAYLAND_DEBUG=1 $MPV --no-config --vo=$VO --pause $CLIP > $D/mpv.log 2>&1" \
  > "$D/scoot.log" 2>&1 &
CPID=$!
for t in $(seq 1 50); do [ -S "$D/scoot.sock" ] && break; sleep 0.1; done
sleep 5
SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" windows > "$D/windows.json" 2>&1
SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" screenshot --out "$D/shot-1.png" > /dev/null 2>&1
sleep 2
SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" screenshot --out "$D/shot-2.png" > /dev/null 2>&1
for s in 1 2; do
  echo -n "$RENDERER/$VO shot-$s: "
  magick "$D/shot-$s.png" -crop 766x960+20+20 +repage \
    -format 'mean=%[fx:mean] std=%[fx:standard_deviation]\n' info:
done
echo "buffers: $(grep -c 'create_buffer(' "$D/mpv.log" 2>/dev/null) shm-pools: $(grep -c 'create_pool(' "$D/mpv.log" 2>/dev/null) params.add: $(grep -c 'params.*\.add(' "$D/mpv.log" 2>/dev/null)"
pkill -f "testsrc-nv12.mkv" 2>/dev/null
sleep 0.3
kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
echo done
