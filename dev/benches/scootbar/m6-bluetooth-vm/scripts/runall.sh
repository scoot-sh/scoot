#!/bin/bash
# The rows of the bluetooth module's cost table. BINS has scootbar-main
# (origin/main), scootbar-off (the branch built without `bluetooth`) and
# scootbar-on (the branch, default features), each a release build
# (lto = "fat", stripped).
BINS=/tmp/bt-bins
HERE=$(dirname "$0")
C=/tmp/bt-bench-run/cfg; mkdir -p $C
printf 'center = []\n' > $C/none.toml
printf 'right = ["clock"]\n' > $C/clock.toml
printf 'left = ["bluetooth"]\n' > $C/bt.toml
printf 'left = ["bluetooth"]\nright = ["clock"]\n' > $C/bt-clock.toml
row() { echo "=== $1"; shift; bash $HERE/measure.sh "$@" 2>&1; }
row "main, no module placed"                    main-none $BINS/scootbar-main $C/none.toml bus
row "branch (bluetooth built), no module placed" on-none $BINS/scootbar-on   $C/none.toml bus
row "branch, bluetooth feature off, no module placed" off-none $BINS/scootbar-off $C/none.toml bus
row "main, clock"                               main-clock $BINS/scootbar-main $C/clock.toml bus
row "branch, bluetooth feature off, clock"      off-clock  $BINS/scootbar-off $C/clock.toml bus
row "branch, bluetooth built and not placed, clock" on-clock $BINS/scootbar-on $C/clock.toml bus
row "bluetooth alone, no bus"                   b-nobus   $BINS/scootbar-on   $C/bt.toml nobus
row "bluetooth alone, bus, no BlueZ"            b-bus     $BINS/scootbar-on   $C/bt.toml bus
row "bluetooth alone, bus, BlueZ idle (adapter on, one device connected)" b-bluez $BINS/scootbar-on $C/bt.toml bluez
row "bluetooth and clock, bus, BlueZ idle"      b-clock   $BINS/scootbar-on   $C/bt-clock.toml bluez
row "bluetooth alone, bus, connect/disconnect storm (10k signals)" b-flood $BINS/scootbar-on $C/bt.toml flood
echo ALLDONE
