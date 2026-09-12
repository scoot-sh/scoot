#!/usr/bin/env bash
# Drives flexwm end to end with no display: start it, open a terminal inside it,
# type into that terminal, and capture the screen.
set -euo pipefail

FLEXWM=${FLEXWM:-/var/cargo-target/debug/flexwm}
SOCKET=${SOCKET:-/run/user/$(id -u)/flexwm-smoke.sock}
SHOT=${SHOT:-/tmp/flexwm-smoke.png}
LOG=${LOG:-/tmp/flexwm-smoke.log}
export FLEXWM_SOCKET="$SOCKET"

rm -f "$SOCKET" "$SHOT" "$LOG"

"$FLEXWM" --headless --width 1200 --height 800 --socket "$SOCKET" >"$LOG" 2>&1 &
compositor=$!
trap 'kill "$compositor" 2>/dev/null || true' EXIT

for _ in $(seq 1 60); do
    [ -S "$SOCKET" ] && break
    sleep 0.1
done
if [ ! -S "$SOCKET" ]; then
    echo "the control socket never appeared; compositor log:"
    tail -20 "$LOG"
    exit 1
fi

echo "--- version ---"
"$FLEXWM" msg version

echo "--- opening a terminal ---"
"$FLEXWM" msg action spawn foot

echo "--- waiting for it to appear as a window ---"
mapped=0
for _ in $(seq 1 100); do
    if "$FLEXWM" msg windows | grep -q '"app_id"'; then
        mapped=1
        break
    fi
    sleep 0.2
done
if [ "$mapped" -ne 1 ]; then
    echo "foot never mapped a window; compositor log:"
    tail -30 "$LOG"
    exit 1
fi
"$FLEXWM" msg windows

echo "--- typing ---"
"$FLEXWM" msg type 'hello from flexwm'
"$FLEXWM" msg wait-idle --quiet-ms 500 --timeout-ms 10000

echo "--- checking the typed text actually reached the client ---"
"$FLEXWM" msg screenshot --out "$SHOT.check1" >/dev/null
"$FLEXWM" msg type 'x'
"$FLEXWM" msg wait-idle --quiet-ms 500 --timeout-ms 10000
"$FLEXWM" msg screenshot --out "$SHOT.check2" >/dev/null
if cmp -s "$SHOT.check1" "$SHOT.check2"; then
    echo "BUG: the screen did not change after typing -- input is not reaching the client"
    echo "(this is the flush_clients bug from 2026-09-11 if it ever comes back)"
    exit 1
fi
rm -f "$SHOT.check1" "$SHOT.check2"
echo "ok: the screen changed after typing"

echo "--- typing text with an embedded newline ---"
# xkb::utf32_to_keysym has no mapping for control characters, so a literal
# '\n' used to fail to find a key at all and abort the rest of the string
# (the 2026-09-12 keysym_for_char bug, if it ever comes back).
"$FLEXWM" msg type "$(printf 'echo one\necho two')"
"$FLEXWM" msg wait-idle --quiet-ms 500 --timeout-ms 10000

echo "--- screenshot ---"
"$FLEXWM" msg screenshot --out "$SHOT"

echo "--- compositor log (tail) ---"
tail -20 "$LOG"
