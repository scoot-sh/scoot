#!/usr/bin/env bash
# Scores scripts/mpv-shm-rate/run.sh output: per screenshot, mean/std over
# the mpv window's content rect (window rect from windows.json, inset 8px to
# stay off the ring). Classification from calibration on known outcomes:
#   black:   std < 0.01  (uniform background, mean ~0.25)
#   partial: 0.01 <= std < 0.30 (striped rows)
#   correct: std >= 0.30 (testsrc2 bars, pixman reference std ~0.33)
# usage: score.sh <outdir>  (prints one line per screenshot + a tally)
set -u
D=$1
tally_b=0; tally_p=0; tally_c=0; tally_x=0
for rundir in "$D"/run-*; do
  [ -d "$rundir" ] || continue
  run=$(basename "$rundir")
  json=$(cat "$rundir/windows.json" 2>/dev/null)
  x=$(echo "$json" | grep -o '"x": [0-9]*' | head -1 | grep -o '[0-9]*')
  y=$(echo "$json" | grep -o '"y": [0-9]*' | head -1 | grep -o '[0-9]*')
  w=$(echo "$json" | grep -o '"width": [0-9]*' | head -1 | grep -o '[0-9]*')
  h=$(echo "$json" | grep -o '"height": [0-9]*' | head -1 | grep -o '[0-9]*')
  [ -n "${x:-}" ] || { echo "$run: NO RECT"; tally_x=$((tally_x+1)); continue; }
  crop="$((w-16))x$((h-16))+$((x+8))+$((y+8))"
  for shot in "$rundir"/shot-*.png; do
    [ -f "$shot" ] || { echo "$run/$(basename "$shot"): MISSING"; tally_x=$((tally_x+1)); continue; }
    stats=$(magick "$shot" -crop "$crop" +repage -format '%[fx:mean],%[fx:standard_deviation]' info: 2>/dev/null)
    mean=${stats%%,*}; std=${stats##*,}
    cls=$(awk -v s="$std" 'BEGIN{print (s<0.01)?"black":((s<0.30)?"partial":"correct")}')
    case $cls in black) tally_b=$((tally_b+1));; partial) tally_p=$((tally_p+1));; correct) tally_c=$((tally_c+1));; esac
    echo "$run/$(basename "$shot" .png): mean=$mean std=$std $cls"
  done
done
echo "TALLY black=$tally_b partial=$tally_p correct=$tally_c missing=$tally_x"
