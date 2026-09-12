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

# The two scenarios below each start their own, separate compositor (a fresh
# --config only means anything at startup), always --headless -- config
# loading is backend-agnostic, so there's no need to repeat these under
# --nested/--tty too, the way the IPC-driven test above deliberately covers
# every backend.

echo "=== config file: a user bind actually takes effect ==="
run_config_bind_test() {
    local socket="/run/user/$(id -u)/flexwm-smoke-config.sock"
    local log="/tmp/flexwm-smoke-config.log"
    local cfg
    cfg=$(mktemp /tmp/flexwm-smoke-config-XXXXXX.toml)
    # super+n is bound to nothing by default (see keybindings.rs) -- an
    # otherwise-unused combo, so this only passes if the config file's bind
    # is what moved focus, not some default binding coincidentally doing it.
    cat >"$cfg" <<'EOF'
[binds]
"super+n" = "focus-column right"
EOF
    rm -f "$socket" "$log"

    "$FLEXWM" --headless --width 1200 --height 800 --socket "$socket" --config "$cfg" \
        >"$log" 2>&1 &
    local pid=$!
    trap 'kill "$pid" 2>/dev/null || true' RETURN
    export FLEXWM_SOCKET="$socket"

    for _ in $(seq 1 60); do
        [ -S "$socket" ] && break
        sleep 0.1
    done
    if [ ! -S "$socket" ]; then
        echo "config-bind test: the control socket never appeared; compositor log:"
        tail -20 "$log"
        return 1
    fi

    "$FLEXWM" msg action spawn foot
    "$FLEXWM" msg action spawn foot
    local mapped=0
    for _ in $(seq 1 100); do
        if [ "$("$FLEXWM" msg windows | jq '.windows | length')" -eq 2 ]; then
            mapped=1
            break
        fi
        sleep 0.2
    done
    if [ "$mapped" -ne 1 ]; then
        echo "config-bind test: the two windows never mapped; compositor log:"
        tail -30 "$log"
        return 1
    fi

    # The second window opens to the right and takes focus (same placement
    # rule the main test above already relies on).
    local right_id left_id
    right_id=$("$FLEXWM" msg windows | jq -r '.windows[] | select(.focused) | .id')
    left_id=$("$FLEXWM" msg windows | jq -r --argjson id "$right_id" '.windows[] | select(.id != $id) | .id')

    # Move left with the default binding first, so the only way super+n can
    # bring focus back to the right column is if it really is bound to
    # focus-column right -- not e.g. a no-op at an already-rightmost column.
    "$FLEXWM" msg key super+h
    "$FLEXWM" msg wait-idle --quiet-ms 300 --timeout-ms 10000
    if [ "$(focused_window)" != "$left_id" ]; then
        echo "config-bind test: super+h did not move focus left; compositor log:"
        tail -30 "$log"
        return 1
    fi

    "$FLEXWM" msg key super+n
    "$FLEXWM" msg wait-idle --quiet-ms 300 --timeout-ms 10000
    if [ "$(focused_window)" != "$right_id" ]; then
        echo "BUG: the config file's super+n -> focus-column right bind did not take effect"
        "$FLEXWM" msg windows
        return 1
    fi
    echo "ok: the config file's super+n bind (focus-column right) moved focus as configured"
}
( run_config_bind_test ) || exit 1

echo "=== config file: a malformed file falls back to defaults instead of blocking startup ==="
run_broken_config_test() {
    local socket="/run/user/$(id -u)/flexwm-smoke-broken.sock"
    local log="/tmp/flexwm-smoke-broken.log"
    local cfg
    cfg=$(mktemp /tmp/flexwm-smoke-broken-XXXXXX.toml)
    printf 'this is not valid toml [[[\n' >"$cfg"
    rm -f "$socket" "$log"

    "$FLEXWM" --headless --width 1200 --height 800 --socket "$socket" --config "$cfg" \
        >"$log" 2>&1 &
    local pid=$!
    trap 'kill "$pid" 2>/dev/null || true' RETURN
    export FLEXWM_SOCKET="$socket"

    for _ in $(seq 1 60); do
        [ -S "$socket" ] && break
        sleep 0.1
    done
    if [ ! -S "$socket" ]; then
        echo "BUG: a malformed --config file prevented startup -- this is the lockout scenario"
        echo "config.rs's failure-semantics rule exists specifically to prevent this"
        tail -30 "$log"
        return 1
    fi
    echo "ok: the compositor started despite a malformed config file"

    if ! grep -qi "could not parse config file" "$log"; then
        echo "BUG: no log message about the malformed config -- the fallback happened with no signal"
        tail -30 "$log"
        return 1
    fi
    echo "ok: the fallback to defaults was logged"

    "$FLEXWM" msg version
    "$FLEXWM" msg action spawn foot
    local mapped=0
    for _ in $(seq 1 100); do
        if [ "$("$FLEXWM" msg windows | jq '.windows | length')" -eq 1 ]; then
            mapped=1
            break
        fi
        sleep 0.2
    done
    if [ "$mapped" -ne 1 ]; then
        echo "BUG: normal operation did not proceed after falling back to defaults"
        tail -30 "$log"
        return 1
    fi
    echo "ok: normal operation (spawning a window) works after the fallback"
}
( run_broken_config_test ) || exit 1
