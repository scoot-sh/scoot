#!/usr/bin/env bash
# GL-client control: es2gears_wayland under each renderer, 6 runs x 2 shots.
# usage: gl-control.sh <outdir>
set -u
D=$1
SCOOT=/var/cargo-target/debug/scoot
SCOOTCTL=/var/cargo-target/debug/scootctl
GEARS=/nix/store/8l6kj814gkkq8yw1qcjw9pr8hdy98wbz-mesa-demos-9.0.0/bin/es2gears_wayland
mkdir -p "$D"
for R in pixman gles; do
  for i in $(seq 1 6); do
    T=$D/$R-$i; mkdir -p "$T"
    rm -f "$T/scoot.sock"
    RUST_LOG=scoot=warn "$SCOOT" --headless --renderer "$R" --socket "$T/scoot.sock" \
      -- sh -c "exec $GEARS > $T/client.log 2>&1" > "$T/scoot.log" 2>&1 &
    CPID=$!
    for t in $(seq 1 50); do [ -S "$T/scoot.sock" ] && break; sleep 0.1; done
    sleep 4
    SCOOT_SOCKET="$T/scoot.sock" "$SCOOTCTL" windows > "$T/windows.json" 2>&1
    json=$(cat "$T/windows.json")
    x=$(echo "$json" | grep -o '"x": [0-9]*' | head -1 | grep -o '[0-9]*')
    y=$(echo "$json" | grep -o '"y": [0-9]*' | head -1 | grep -o '[0-9]*')
    w=$(echo "$json" | grep -o '"width": [0-9]*' | head -1 | grep -o '[0-9]*')
    h=$(echo "$json" | grep -o '"height": [0-9]*' | head -1 | grep -o '[0-9]*')
    crop="$((w-16))x$((h-16))+$((x+8))+$((y+8))"
    for s in 1 2; do
      SCOOT_SOCKET="$T/scoot.sock" "$SCOOTCTL" screenshot --out "$T/shot-$s.png" > /dev/null 2>&1
      stats=$(magick "$T/shot-$s.png" -crop "$crop" +repage \
        -format '%[fx:mean],%[fx:standard_deviation]' info: 2>/dev/null)
      echo "$R-$i/shot-$s: $stats"
      sleep 0.5
    done
    pkill -x es2gears_wayland 2>/dev/null
    sleep 0.3
    kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
  done
done
echo done
