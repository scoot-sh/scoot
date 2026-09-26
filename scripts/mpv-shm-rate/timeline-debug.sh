#!/usr/bin/env bash
# Timeline with WAYLAND_DEBUG: correlates compositor screenshots with actual
# client commit traffic. usage: timeline-debug.sh <pixman|gles> <outdir>
set -u
RENDERER=$1; D=$2
SCOOT=/var/cargo-target/debug/scoot
SCOOTCTL=/var/cargo-target/debug/scootctl
MPV=/nix/store/dddlcx6mfwrjxlscbf1rhxswj17987vs-mpv-0.41.0/bin/mpv
CLIP=/home/dev/evidence/gdf/testsrc-nv12.mkv
mkdir -p "$D"
rm -f "$D/scoot.sock"
RUST_LOG=scoot=debug "$SCOOT" --headless --renderer "$RENDERER" --socket "$D/scoot.sock" \
  -- sh -c "exec env WAYLAND_DEBUG=1 $MPV --no-config --vo=gpu --gpu-context=wayland --loop $CLIP > $D/mpv.log 2>&1" \
  > "$D/scoot.log" 2>&1 &
CPID=$!
for t in $(seq 1 50); do [ -S "$D/scoot.sock" ] && break; sleep 0.1; done
json=$(cat /dev/null; SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" windows 2>&1)
echo "$json" > "$D/windows.json"
x=$(echo "$json" | grep -o '"x": [0-9]*' | head -1 | grep -o '[0-9]*')
y=$(echo "$json" | grep -o '"y": [0-9]*' | head -1 | grep -o '[0-9]*')
w=$(echo "$json" | grep -o '"width": [0-9]*' | head -1 | grep -o '[0-9]*')
h=$(echo "$json" | grep -o '"height": [0-9]*' | head -1 | grep -o '[0-9]*')
crop="$((w-16))x$((h-16))+$((x+8))+$((y+8))"
for t in $(seq 3 3 30); do
  sleep 3
  SCOOT_SOCKET="$D/scoot.sock" "$SCOOTCTL" screenshot --out "$D/t-$t.png" > /dev/null 2>&1
  commits=$(grep -cE 'wl_surface#[0-9]+\.commit\(\)' "$D/mpv.log" 2>/dev/null || echo 0)
  stats=$(magick "$D/t-$t.png" -crop "$crop" +repage -format '%[fx:mean],%[fx:standard_deviation]' info: 2>/dev/null)
  std=${stats##*,}
  cls=$(awk -v s="$std" 'BEGIN{print (s<0.01)?"black":((s<0.30)?"partial":"correct")}')
  echo "t=${t}s commits=$commits std=$std $cls"
done
echo "final commit count: $(grep -cE 'wl_surface#[0-9]+\.commit\(\)' "$D/mpv.log")"
echo "attaches: $(grep -cE 'wl_surface#[0-9]+\.attach\(' "$D/mpv.log")"
echo "releases: $(grep -cE 'wl_buffer#[0-9]+\.release\(\)' "$D/mpv.log")"
pkill -f "testsrc-nv12.mkv" 2>/dev/null
sleep 0.3
kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
echo done
