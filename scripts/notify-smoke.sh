#!/bin/sh
# notify-smoke.sh: prove an installed scootnotify serves Notify.
#
# Usage: notify-smoke.sh EXAMPLE_CONFIG
#
# Starts `scootnotify daemon` headless on this session bus (the caller
# provides it, e.g. `dbus-run-session -- scripts/notify-smoke.sh ...`),
# waits for the name, sends one Notify, and checks `scootnotify msg
# list --json` shows it. HOME and XDG_RUNTIME_DIR come from the caller
# (own temp dirs, so the control socket and the id file land nowhere
# shared). The daemon is killed by PID on exit, never by pattern.
#
# Needs: scootnotify and gdbus on PATH, a session bus above. The Notify
# below passes its eight arguments separately (one gdbus arg each):
# gdbus would send a single `(...)` tuple as one struct argument, whose
# signature never matches the method.
set -eu

example=${1:?usage: notify-smoke.sh EXAMPLE_CONFIG}

scootnotify daemon --config "$example" &
daemon=$!
trap 'kill "$daemon" 2>/dev/null || true' EXIT
gdbus wait --session --timeout 10 org.freedesktop.Notifications
gdbus call --session --dest org.freedesktop.Notifications \
    --object-path /org/freedesktop/Notifications \
    --method org.freedesktop.Notifications.Notify \
    'smoke' 'uint32 0' '' 'hello' 'world' '@as []' '@a{sv} {}' 'int32 -1'
out=$(scootnotify msg list --json)
echo "$out" | grep -q hello
echo 'ok: scootnotify serves Notify and msg list on a session bus'
