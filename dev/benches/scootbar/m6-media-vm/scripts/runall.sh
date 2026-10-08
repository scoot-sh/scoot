#!/bin/bash
# The rows of the media module's cost table. BINS has scootbar-main (origin/main),
# scootbar-off (the branch built without `media`) and scootbar-on (the branch, default
# features), each a release build (lto = "fat", stripped).
BINS=/tmp/media-bins
HERE=$(dirname "$0")
C=/tmp/media-bench-run/cfg; mkdir -p $C
printf 'center = []\n' > $C/none.toml
printf 'right = ["clock"]\n' > $C/clock.toml
printf 'left = ["media"]\n' > $C/media.toml
printf 'left = ["media"]\nright = ["clock"]\n' > $C/media-clock.toml
row() { echo "=== $1"; shift; bash $HERE/measure.sh "$@" 2>&1; }
row "main, no module placed"                      main-none   $BINS/scootbar-main $C/none.toml bus
row "branch (media built), no module placed"      on-none     $BINS/scootbar-on   $C/none.toml bus
row "branch, media feature off, no module placed" off-none    $BINS/scootbar-off  $C/none.toml bus
row "main, clock"                                 main-clock  $BINS/scootbar-main $C/clock.toml bus
row "branch, media feature off, clock"            off-clock   $BINS/scootbar-off  $C/clock.toml bus
row "branch, media built and not placed, clock"   on-clock    $BINS/scootbar-on   $C/clock.toml bus
row "media alone, no bus"                         m-nobus     $BINS/scootbar-on   $C/media.toml nobus
row "media alone, bus, no player"                 m-bus       $BINS/scootbar-on   $C/media.toml bus
row "media alone, bus, one paused stub"           m-paused    $BINS/scootbar-on   $C/media.toml paused
row "media alone, bus, one playing stub"          m-playing   $BINS/scootbar-on   $C/media.toml playing
row "media alone, bus, eight playing stubs"       m-eight     $BINS/scootbar-on   $C/media.toml eight
row "media alone, bus, one real mpv playing a file" m-mpv     $BINS/scootbar-on   $C/media.toml mpv
row "media alone, bus, mpv playing its lavfi sine (re-sends its metadata each second)" m-mpvlavfi $BINS/scootbar-on $C/media.toml mpvlavfi
row "media and clock, bus, one playing stub"      m-clock     $BINS/scootbar-on   $C/media-clock.toml playing
echo ALLDONE
