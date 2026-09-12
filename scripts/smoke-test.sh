#!/usr/bin/env bash
# Drives flexwm end to end over IPC only: start it, open a terminal inside it,
# type into that terminal, and capture the screen. Backend-agnostic by design
# -- IPC input/introspection works the same regardless of how the compositor
# presents itself, so this is the regression net for every backend, not just
# --headless.
#
# MODE selects the backend (default --headless). For --nested, run this
# script itself inside a host compositor, e.g.:
#   WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
#     cage -- env MODE=--nested scripts/smoke-test.sh
# so flexwm's own Connection::connect_to_env() (nested.rs) finds cage's
# WAYLAND_DISPLAY when it starts, not this shell's.
set -euo pipefail

MODE=${MODE:---headless}
FLEXWM=${FLEXWM:-/var/cargo-target/debug/flexwm}
SOCKET=${SOCKET:-/run/user/$(id -u)/flexwm-smoke.sock}
SHOT=${SHOT:-/tmp/flexwm-smoke.png}
LOG=${LOG:-/tmp/flexwm-smoke.log}
export FLEXWM_SOCKET="$SOCKET"

rm -f "$SOCKET" "$SHOT" "$LOG"

"$FLEXWM" "$MODE" --width 1200 --height 800 --socket "$SOCKET" >"$LOG" 2>&1 &
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

focused_window() {
    "$FLEXWM" msg windows | jq -r '.windows[] | select(.focused) | .id'
}

echo "--- opening a second terminal, to exercise keybindings ---"
"$FLEXWM" msg action spawn foot
mapped=0
for _ in $(seq 1 100); do
    if [ "$("$FLEXWM" msg windows | jq '.windows | length')" -eq 2 ]; then
        mapped=1
        break
    fi
    sleep 0.2
done
if [ "$mapped" -ne 1 ]; then
    echo "the second foot never mapped a window; compositor log:"
    tail -30 "$LOG"
    exit 1
fi
# A new column opens to the right and takes focus (see world.rs's placement).
second_id=$(focused_window)
first_id=$("$FLEXWM" msg windows | jq -r --argjson id "$second_id" '.windows[] | select(.id != $id) | .id')

echo "--- a bare 'h' has no binding: it types into the focused terminal, focus stays put ---"
"$FLEXWM" msg key h
"$FLEXWM" msg wait-idle --quiet-ms 300 --timeout-ms 10000
if [ "$(focused_window)" != "$second_id" ]; then
    echo "BUG: a bare 'h' (no modifier) moved focus -- keybindings must not fire without Super"
    exit 1
fi
echo "ok: focus unchanged by a bare 'h'"

echo "--- super+h moves focus to the column on the left (Action::FocusColumn) ---"
"$FLEXWM" msg key super+h
"$FLEXWM" msg wait-idle --quiet-ms 300 --timeout-ms 10000
if [ "$(focused_window)" != "$first_id" ]; then
    echo "BUG: super+h did not move focus to the other column; keybindings may be broken"
    "$FLEXWM" msg windows
    exit 1
fi
echo "ok: super+h moved focus from window $second_id to window $first_id"

echo "--- screenshot ---"
"$FLEXWM" msg screenshot --out "$SHOT"

echo "--- compositor log (tail) ---"
tail -20 "$LOG"
