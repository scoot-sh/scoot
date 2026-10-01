#!/usr/bin/env bash
# quick.sh SCOOT SCOOTBAR OUTDIR [settle]   one scootbar against headless scoot; dumps /proc readings
SCOOT=$1; BAR=$2; OUT=$3; SETTLE=${4:-20}
FONT=/nix/store/33wfvwgr4vy0gaznf3wafla7mfni1znw-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf
rm -rf $OUT; mkdir -p $OUT; D=$(mktemp -d /tmp/sbq.XXXXXX); chmod 700 $D
export XDG_RUNTIME_DIR=$D HOME=$D/h XDG_CONFIG_HOME=$D/c XDG_CACHE_HOME=$D/ca XDG_STATE_HOME=$D/s; mkdir -p $D/h $D/c $D/ca $D/s
: > $D/scoot.toml
env -u WAYLAND_DISPLAY $SCOOT --headless --width 1920 --height 1080 --outputs 1 --socket $D/s.sock --config $D/scoot.toml >$D/scoot.log 2>&1 &
SP=$!
for i in $(seq 100); do ls $D | grep -q '^wayland-[0-9]*$' && [ -S $D/s.sock ] && break; sleep 0.05; done
export WAYLAND_DISPLAY=$(ls $D | grep '^wayland-[0-9]*$' | head -1)
$WRAP $BAR daemon --font $FONT --font-size 14 --height 26 --background '#1e1e2e' --foreground '#cdd6f4' --right clock --clock-format '%a %d %b %H:%M' >$D/bar.log 2>&1 &
BP=$!
sleep 0.5; cat /proc/$BP/stat > $OUT/stat.early; grep -E 'Rss|Pss' /proc/$BP/smaps_rollup > $OUT/rollup.early
sleep $SETTLE
for f in smaps smaps_rollup stat status maps; do cat /proc/$BP/$f > $OUT/$f; done; mv $OUT/smaps_rollup $OUT/rollup
R=$(grep 'r-xp.*scootbar$' /proc/$BP/maps | head -1 | cut -d' ' -f1); A=$((16#${R%-*})); Z=$((16#${R#*-})); echo $A $Z > $OUT/text.range
dd if=/proc/$BP/pagemap bs=8 skip=$((A/16384)) count=$(((Z-A)/16384)) of=$OUT/text.pagemap 2>/dev/null
cat $D/bar.log >> $OUT/bar.log
kill $BP $SP 2>/dev/null; wait 2>/dev/null; rm -rf $D
