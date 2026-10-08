#!/bin/sh
# Reproducible ginger-night preview: headless scoot (this directory's
# example files) + bar + two terminals, captured over scoot's own IPC
# screenshot path (`scoot msg screenshot`, the same path the docs' own
# screenshots use). Run on the Asahi M2 with the installed set
# (scoot/scootbar/scootbg/foot, ipc protocol 10):
#
#   sh docs/examples/ginger-night/screenshot.sh 1920 1080 /tmp/ginger-night-preview.png
#
# Composition mirrors the moonrise preview: a floating bar over two
# translucent terminal columns with the wallpaper showing through. The
# left terminal prints the look's palette as swatches; the right one
# lists this directory. `load.sh`/`cpu.sh` ride PATH for the bar's
# command-fed modules. Nothing here rewrites the committed files.
set -eu
W=${1:?width}; H=${2:?height}; OUT=${3:?out png}
G=$(cd "$(dirname "$0")" && pwd)
FX=$HOME/fx/catlook-shot
RT=$(mktemp -d "$FX/rt-XXXXXX"); chmod 700 "$RT"
export XDG_RUNTIME_DIR=$RT
export SCOOT_SOCKET=$RT/scoot.sock
mkdir -p "$FX/home"; export HOME=$FX/home
export PATH="$G:/etc/profiles/per-user/steve/bin:/run/current-system/sw/bin:$PATH"

scoot --headless --width "$W" --height "$H" --socket "$SCOOT_SOCKET" \
  --config "$G/scoot.toml" >"$RT/scoot.log" 2>&1 &
SCOOT_PID=$!
cleanup() { kill $SCOOT_PID 2>/dev/null || true; }
trap cleanup EXIT

for i in $(seq 1 150); do
  [ -S "$SCOOT_SOCKET" ] && scoot msg outputs >/dev/null 2>&1 && break
  sleep 0.1
done
scoot msg outputs

# The bar has no fontconfig: under a scratch HOME its usual-file lookup
# finds nothing, so resolve the look's bar face explicitly (CLI wins over
# the file, as usual; the committed bar.toml is untouched).
BAR_FONT=$(fc-match --format='%{file}\n' 'DroidSansM Nerd Font Propo' | head -1)
scoot msg action spawn scootbar daemon --font "$BAR_FONT" --config "$G/bar.toml"
scoot msg action spawn foot --config "$G/foot.ini" sh -c "printf '%b' '\033[48;2;14;14;14m  surface #0E0E0E  \033[0m \033[48;2;245;234;214m  ink #F5EAD6  \033[0m\n\033[48;2;229;127;41m  accent #E57F29  \033[0m \033[48;2;255;154;48m  ring #FF9A30  \033[0m\n\033[48;2;255;177;75m  glow #FFB14B  \033[0m \033[48;2;224;90;78m  ember #E05A4E  \033[0m\n\033[48;2;143;122;99m  mist #8F7A63  \033[0m \033[48;2;78;41;19m  hush #4E2913  \033[0m\n'; exec sleep 300"
scoot msg action spawn foot --config "$G/foot.ini" sh -c "ls -1 \"$G\"; exec sleep 300"

# Settle: wallpaper decoded, bar modules polled, terminals mapped.
sleep 6
scoot msg windows

scoot msg screenshot --out "$OUT"
ls -l "$OUT"

scoot msg action quit || kill $SCOOT_PID
trap - EXIT
mkdir -p "$FX/shot-logs"
TAG=$(basename "$OUT" .png)
for f in "$RT"/*.log; do cp "$f" "$FX/shot-logs/$TAG-$(basename "$f")"; done
rm -rf "$RT"
echo "ok: $OUT"
