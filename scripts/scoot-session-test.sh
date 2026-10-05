#!/bin/sh
# scoot-session-test.sh: a stub harness for `resources/scoot-session`.
#
# Runs the launcher against fake `systemctl --user`, `flock`,
# `dbus-update-activation-environment`, `timeout`, `date`, `sleep` and
# `scoot` binaries on PATH, with unit state in files, and asserts the
# session ordering the Asahi bug was about:
#
#   - `scoot-session.target` is started only AFTER the scoped display
#     import, and starting it is what reaches `graphical-session.target`
#     (a `WantedBy` probe unit with
#     `ConditionEnvironment=WAYLAND_DISPLAY` starts on a real login and
#     is skipped when the display is missing -- the swayidle shape);
#   - starting `scoot.service` alone never reaches any session target;
#   - a readiness-deadline failure stops the service and starts
#     nothing (leaves nothing active);
#   - a stale session (units active, IPC silent, lock free) is healed
#     -- the session target stopped too -- and the login proceeds;
#   - a lock held by another launcher, or another desktop's graphical
#     target with IPC silent, refuses without touching any unit;
#   - without the dbus tool the display still reaches the user manager
#     through `import-environment`.
#
# Usage: scripts/scoot-session-test.sh [--launcher PATH] [--keep DIR]
# Defaults to the tree's `resources/scoot-session`. Exit nonzero on the
# first failed assert; prints `ok` lines and a final `N/N asserts`
# count. POSIX sh (runs on macOS and Linux); sockets are real unix
# sockets (python3), since the launcher tests them with `-S`.
set -u

LAUNCHER="$(dirname "$0")/../resources/scoot-session"
KEEP=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --launcher) LAUNCHER="$2"; shift 2 ;;
        --keep) KEEP="$2"; shift 2 ;;
        *) echo "usage: $0 [--launcher PATH] [--keep DIR]" >&2; exit 2 ;;
    esac
done

PASS=0
TOTAL=0
ok() { TOTAL=$((TOTAL + 1)); PASS=$((PASS + 1)); echo "ok: $1"; }
bad() { TOTAL=$((TOTAL + 1)); echo "FAIL: $1" >&2; for p in $(jobs -p 2>/dev/null); do kill "$p" 2>/dev/null || true; done; [ -n "$KEEP" ] || rm -rf "$ROOT"; exit 1; }
[ -x "$LAUNCHER" ] || { echo "launcher not executable: $LAUNCHER" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "needs python3 (unix sockets)" >&2; exit 2; }

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/scoot-session-test.XXXXXX")"
FAKES="$ROOT/fakes"
# Prepended, never replaced: NixOS keeps its tools outside /usr/bin.
BASE_PATH="$PATH"
mkdir -p "$FAKES"
REAL_SLEEP="$(command -v sleep)"
REAL_DATE="$(command -v date)"

# --- fake binaries -------------------------------------------------
# Every fake records `index: name argv...` to $HARNESS_STATE/calls.log
# and keeps unit state under $HARNESS_STATE/ (active/<unit> holding the
# ActiveState, `env-manager` as KEY=VALUE lines, `swayidle` as
# active|skipped|inactive). The fakes encode real systemd semantics:
# starting scoot-session.target pulls in graphical-session.target
# (BindsTo); starting scoot.service pulls in nothing (PartOf only);
# starting scoot-shutdown.target stops every session target.

cat >"$FAKES/systemctl" <<'EOF'
#!/bin/sh
# fake systemctl --user. Only the subcommands scoot-session uses.
set -u
S="$HARNESS_STATE"
log() {
    n=0
    if [ -f "$S/calls-seq" ]; then n=$(cat "$S/calls-seq"); fi
    n=$((n + 1)); printf '%s' "$n" >"$S/calls-seq"
    printf '%s: systemctl' "$n" >>"$S/calls.log"
    for a in "$@"; do printf ' %s' "$a" >>"$S/calls.log"; done
    printf '\n' >>"$S/calls.log"
}
[ "${1:-}" = "--user" ] && shift
log "$@"
state_of() { f="$S/active/$1"; [ -f "$f" ] && cat "$f" || printf 'inactive'; }
set_state() { printf '%s' "$2" >"$S/active/$1"; }
env_upsert() {
    k="$1"; v="$2"
    grep -v "^$k=" "$S/env-manager" 2>/dev/null >"$S/env-manager.tmp" || true
    printf '%s=%s\n' "$k" "$v" >>"$S/env-manager.tmp"
    mv "$S/env-manager.tmp" "$S/env-manager"
}
env_unset() {
    k="$1"
    grep -v "^$k=" "$S/env-manager" 2>/dev/null >"$S/env-manager.tmp" || true
    mv "$S/env-manager.tmp" "$S/env-manager"
}
env_has() { grep -q "^$1=" "$S/env-manager" 2>/dev/null; }
case "${1:-}" in
    list-units) exit 0 ;;
    show)
        shift
        prop=""; value=0; unit=""
        for a in "$@"; do
            case "$a" in
                -p) prop_next=1 ;;
                ActiveState) [ "${prop_next:-0}" = 1 ] && prop="ActiveState"; prop_next=0 ;;
                ExecMainStatus) [ "${prop_next:-0}" = 1 ] && prop="ExecMainStatus"; prop_next=0 ;;
                --property) prop_next=1 ;;
                --value) value=1 ;;
                -*) ;;
                *) [ "$value" = 1 ] && [ -z "$unit" ] && unit="$a" ;;
            esac
        done
        case "$prop" in
            ActiveState) printf '%s\n' "$(state_of "$unit")" ;;
            ExecMainStatus) printf '0\n' ;;
            *) printf '\n' ;;
        esac
        exit 0 ;;
    show-environment) cat "$S/env-manager" 2>/dev/null; exit 0 ;;
    set-environment)
        kv="${2:-}"; k="${kv%%=*}"; v="${kv#*=}"
        env_upsert "$k" "$v"; exit 0 ;;
    unset-environment) env_unset "${2:-}"; exit 0 ;;
    import-environment)
        shift
        if [ "$#" -eq 0 ]; then
            while IFS='=' read -r k v; do
                case "$k" in ''|*[!A-Za-z0-9_]* ) continue ;; esac
                env_upsert "$k" "$v"
            done <<ALLENV
$(env)
ALLENV
        else
            for n in "$@"; do
                eval "v=\${$n:-__HARNESS_UNSET__}"
                [ "$v" = "__HARNESS_UNSET__" ] || env_upsert "$n" "$v"
            done
        fi
        exit 0 ;;
    start|stop)
        op="$1"; shift
        unit=""
        for a in "$@"; do case "$a" in -*) continue ;; *) unit="$a" ;; esac; done
        case "$unit" in
            scoot-session.target)
                if [ "$op" = start ]; then
                    set_state "$unit" active
                    set_state graphical-session.target active
                    # The swayidle shape: WantedBy the graphical target,
                    # gated on the display being in the manager already.
                    if env_has WAYLAND_DISPLAY; then
                        printf 'active' >"$S/swayidle"
                        printf '%s' "$(cat "$S/env-manager")" >"$S/env-at-graphical"
                    else
                        printf 'skipped' >"$S/swayidle"
                    fi
                else
                    set_state "$unit" inactive
                    set_state scoot.service inactive
                fi ;;
            scoot-shutdown.target)
                if [ "$op" = start ]; then
                    set_state scoot-session.target inactive
                    set_state graphical-session.target inactive
                    set_state graphical-session-pre.target inactive
                    set_state scoot.service inactive
                fi ;;
            *) set_state "$unit" "$([ "$op" = start ] && printf active || printf inactive)" ;;
        esac
        exit 0 ;;
    reset-failed)
        [ "$(state_of "${2:-}")" = "failed" ] && set_state "${2:-}" inactive
        exit 0 ;;
    *) exit 0 ;;
esac
EOF

cat >"$FAKES/dbus-update-activation-environment" <<'EOF'
#!/bin/sh
set -u
S="$HARNESS_STATE"
n=0
if [ -f "$S/calls-seq" ]; then n=$(cat "$S/calls-seq"); fi
n=$((n + 1)); printf '%s' "$n" >"$S/calls-seq"
printf '%s: dbus-update-activation-environment' "$n" >>"$S/calls.log"
for a in "$@"; do printf ' %s' "$a" >>"$S/calls.log"; done
printf '\n' >>"$S/calls.log"
if [ "${DBUS_FAIL:-0}" = 1 ]; then exit 1; fi
[ "${1:-}" = "--systemd" ] && shift
[ "${1:-}" = "--all" ] && shift && {
    while IFS='=' read -r k v; do
        case "$k" in ''|*[!A-Za-z0-9_]* ) continue ;; esac
        grep -v "^$k=" "$S/env-manager" 2>/dev/null >"$S/env-manager.tmp" || true
        printf '%s=%s\n' "$k" "$v" >>"$S/env-manager.tmp"
        mv "$S/env-manager.tmp" "$S/env-manager"
    done <<ALLENV
$(env)
ALLENV
    exit 0
}
for var in "$@"; do
    case "$var" in -*) continue ;; esac
    eval "v=\${$var:-__HARNESS_UNSET__}"
    [ "$v" = "__HARNESS_UNSET__" ] && continue
    grep -v "^$var=" "$S/env-manager" 2>/dev/null >"$S/env-manager.tmp" || true
    printf '%s=%s\n' "$var" "$v" >>"$S/env-manager.tmp"
    mv "$S/env-manager.tmp" "$S/env-manager"
done
exit 0
EOF

# The compositor: `scoot msg version` answers once $ANSWER_AFTER calls
# happened, creating the session's new socket as it comes up.
cat >"$FAKES/scoot" <<'EOF'
#!/bin/sh
set -u
S="$HARNESS_STATE"
[ "${1:-}" = "msg" ] && [ "${2:-}" = "version" ] || exit 1
n=0
[ -f "$S/version-calls" ] && n=$(cat "$S/version-calls")
n=$((n + 1)); printf '%s' "$n" >"$S/version-calls"
if [ "$n" -ge "${ANSWER_AFTER:-1}" ]; then
    if [ ! -e "$XDG_RUNTIME_DIR/wayland-100" ]; then
        python3 -c 'import socket; s = socket.socket(socket.AF_UNIX); s.bind("'"$XDG_RUNTIME_DIR"'/wayland-100")' \
            && printf 'socket' >"$S/socket-made"
    fi
    exit 0
fi
exit 1
EOF

cat >"$FAKES/flock" <<'EOF'
#!/bin/sh
# mkdir-backed `flock -n FD`: the driver pre-holding the directory is
# "the lock held by another launcher".
if [ "${1:-}" = "-n" ]; then
    if mkdir "$XDG_RUNTIME_DIR/harness-flock" 2>/dev/null; then exit 0; else exit 1; fi
fi
exit 0
EOF

cat >"$FAKES/timeout" <<'EOF'
#!/bin/sh
# `timeout SECONDS cmd...`: drop the duration, run the command.
shift
exec "$@"
EOF

cat >"$FAKES/date" <<'EOF'
#!/bin/sh
# Fake clock for `date +%s` (+1 per call, so deadlines pass fast);
# everything else passes through.
if [ "${1:-}" = "+%s" ]; then
    n=1000000
    [ -f "$HARNESS_STATE/fake-now" ] && n=$(cat "$HARNESS_STATE/fake-now")
    n=$((n + 1)); printf '%s' "$n" >"$HARNESS_STATE/fake-now"
    printf '%s\n' "$n"
else
    exec DATE_BIN placeholder
fi
EOF
sed -i '' "s|exec DATE_BIN placeholder|exec $REAL_DATE \"\$@\"|" "$FAKES/date" 2>/dev/null \
    || sed -i "s|exec DATE_BIN placeholder|exec $REAL_DATE \"\$@\"|" "$FAKES/date"

cat >"$FAKES/sleep" <<'EOF'
#!/bin/sh
exit 0
EOF

chmod +x "$FAKES"/*
SCOOT_FAKE="$FAKES/scoot"

mksock() {
    # A real unix socket at $XDG_RUNTIME_DIR/$1.
    python3 -c 'import socket,sys; s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$XDG_RUNTIME_DIR/$1"
}

# Fresh per-test state: empty manager env, all units inactive.
new_test() {
    T="$ROOT/t$1"
    mkdir -p "$T/state/active" "$T/rt"
    export HARNESS_STATE="$T/state" XDG_RUNTIME_DIR="$T/rt"
    export PATH="$FAKES:$BASE_PATH"
    : >"$HARNESS_STATE/calls.log"
    printf 'inactive' >"$HARNESS_STATE/swayidle"
    : >"$HARNESS_STATE/env-manager"
    rm -f "$HARNESS_STATE/calls-seq" "$HARNESS_STATE/version-calls" \
        "$HARNESS_STATE/fake-now" "$HARNESS_STATE/socket-made" \
        "$HARNESS_STATE/env-at-graphical"
    export ANSWER_AFTER=3 DBUS_FAIL=0
    mksock wayland-99
    export SCOOT_BIN="$SCOOT_FAKE"
}

state_of() { f="$HARNESS_STATE/active/$1"; [ -f "$f" ] && cat "$f" || printf 'inactive'; }
set_state() { printf '%s' "$2" >"$HARNESS_STATE/active/$1"; }
env_has() { grep -q "^$1=" "$HARNESS_STATE/env-manager" 2>/dev/null; }
call_index() { grep -n -F "$1" "$HARNESS_STATE/calls.log" | head -1 | cut -d: -f1; }

# Run the launcher in the background; the caller drives the session
# (flips service state to end it) and waits. The exit status lands in
# $T/exit-code (read with wait_exit): this avoids `wait`, whose
# behavior on background children varies across shells.
start_launcher() {
    ( "$LAUNCHER" >"$T/stdout.log" 2>"$T/stderr.log"; printf '%s' "$?" >"$T/exit-code" ) &
    echo "$!" >"$T/pid"
}
wait_exit() {
    # wait_exit <pidfile-test-dir> : up to ~15 s, prints the exit code.
    i=0
    while [ "$i" -lt 150 ]; do
        [ -f "$T/exit-code" ] && { cat "$T/exit-code"; return 0; }
        "$REAL_SLEEP" 0.1
        i=$((i + 1))
    done
    return 1
}
wait_for() {
    # wait_for <file-content> <file> : up to ~10 s.
    i=0
    while [ "$i" -lt 100 ]; do
        [ -f "$2" ] && [ "$(cat "$2")" = "$1" ] && return 0
        "$REAL_SLEEP" 0.1
        i=$((i + 1))
    done
    return 1
}
wait_log() {
    # wait_log <fixed-string> : up to ~10 s.
    i=0
    while [ "$i" -lt 100 ]; do
        grep -q -F "$1" "$HARNESS_STATE/calls.log" 2>/dev/null && return 0
        "$REAL_SLEEP" 0.1
        i=$((i + 1))
    done
    return 1
}
end_session() {
    # The compositor exits: the watch loop sees it, runs cleanup.
    set_state scoot.service inactive
    wait_exit
}
kill_launcher() {
    kill "$1" 2>/dev/null
    i=0
    while kill -0 "$1" 2>/dev/null && [ "$i" -lt 50 ]; do "$REAL_SLEEP" 0.1; i=$((i + 1)); done
}

# --- T1: happy path ------------------------------------------------
new_test 1
start_launcher
wait_log "start scoot-session.target" || bad "T1: launcher never started scoot-session.target"
i_scoped="$(call_index 'dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP')"
[ -n "$i_scoped" ] || bad "T1: no scoped display import before the session start"
i_session="$(call_index 'start scoot-session.target')"
[ "$i_session" -gt "$i_scoped" ] || bad "T1: session target started before the display import"
ok "T1: session target starts after the scoped display import"
i_service="$(call_index 'start scoot.service')"
[ -n "$i_service" ] && [ "$i_service" -lt "$i_session" ] || bad "T1: service not started before the session target"
ok "T1: service starts first, session target after the import"
[ "$(cat "$HARNESS_STATE/swayidle")" = "active" ] || bad "T1: ConditionEnvironment probe unit did not start (got $(cat "$HARNESS_STATE/swayidle"))"
grep -q '^WAYLAND_DISPLAY=wayland-100$' "$HARNESS_STATE/env-at-graphical" || bad "T1: display missing from the manager when the graphical target was reached"
ok "T1: ConditionEnvironment unit starts with the display in the manager"
if grep -q -F "start graphical-session.target" "$HARNESS_STATE/calls.log"; then
    bad "T1: launcher starts graphical-session.target directly (RefuseManualStart)"
fi
ok "T1: graphical target reached by dependency only, never started directly"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T1: launcher exit $RC, expected 0"
[ "$(state_of scoot.service)" = "inactive" ] && [ "$(state_of scoot-session.target)" = "inactive" ] \
    && [ "$(state_of graphical-session.target)" = "inactive" ] \
    || bad "T1: units leak after quit (service=$(state_of scoot.service) session=$(state_of scoot-session.target) graphical=$(state_of graphical-session.target))"
ok "T1: quit stops service, session and graphical targets (nothing leaks)"
env_has WAYLAND_DISPLAY && bad "T1: display leaks into the manager after quit"
ok "T1: manager environment restored on the way out"

# --- T2: readiness deadline leaves nothing active -------------------
new_test 2
export ANSWER_AFTER=999999
start_launcher
RC="$(wait_exit)" || bad "T2: launcher never exited past the deadline"
[ "$RC" != "0" ] || bad "T2: deadline run exited 0"
ok "T2: readiness deadline fails the login (exit $RC)"
grep -q -F "start scoot-session.target" "$HARNESS_STATE/calls.log" \
    && bad "T2: session target started on a failed login"
ok "T2: session target never started on a failed login"
[ "$(state_of scoot.service)" = "inactive" ] || bad "T2: service left active after the deadline"
ok "T2: deadline stops the service (nothing active)"
env_has WAYLAND_DISPLAY && bad "T2: display leaks into the manager after the deadline"
ok "T2: manager environment restored after the deadline"

# --- T3: stale session is healed, login proceeds --------------------
new_test 3
set_state scoot.service active
set_state scoot-session.target active
start_launcher
wait_log "start scoot-session.target" || bad "T3: healed login never reached the session target"
grep -q -F "stop scoot-session.target" "$HARNESS_STATE/calls.log" \
    || bad "T3: stale clear did not stop the session target"
i_stop="$(call_index 'stop scoot-session.target')"
i_restart="$(call_index 'start scoot.service')"
[ "$i_stop" -lt "$i_restart" ] || bad "T3: stale stop not before the fresh service start"
ok "T3: stale units cleared (session target stopped) and the login proceeds"
[ "$(cat "$HARNESS_STATE/swayidle")" = "active" ] || bad "T3: healed login has no display-gated units"
ok "T3: healed login reaches the graphical target with the display"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T3: healed launcher exit $RC"
ok "T3: healed session quits cleanly"

# --- T4: another launcher holds the lock: refuse at once ------------
new_test 4
mkdir "$XDG_RUNTIME_DIR/harness-flock"
start_launcher
RC="$(wait_exit)" || bad "T4: refusing launcher never exited"
[ "$RC" != "0" ] || bad "T4: second launcher exited 0"
if grep -q -F "start scoot.service" "$HARNESS_STATE/calls.log" \
    || grep -q -F "start scoot-session.target" "$HARNESS_STATE/calls.log"; then
    bad "T4: refusing launcher touched units"
fi
ok "T4: lock held elsewhere refuses without touching units (exit $RC)"

# --- T5: another desktop's graphical target: refuse -----------------
new_test 5
set_state graphical-session.target active
export ANSWER_AFTER=999999
start_launcher
RC="$(wait_exit)" || bad "T5: refusing launcher never exited"
[ "$RC" != "0" ] || bad "T5: login over another desktop exited 0"
[ "$(state_of graphical-session.target)" = "active" ] || bad "T5: refusing login stopped another session's target"
if grep -q -F "stop " "$HARNESS_STATE/calls.log"; then bad "T5: refusing login stopped units"; fi
ok "T5: foreign graphical target refuses, nothing stopped (exit $RC)"

# --- T6: no dbus tool: manager half still imported ------------------
# A closed PATH: the fakes minus the dbus tool, plus the bare system
# tools the launcher and driver call -- so `command -v
# dbus-update-activation-environment` fails even where the host has
# one, while nothing else breaks.
new_test 6
mkdir -p "$T/nodbus"
for f in "$FAKES"/*; do
    [ "${f##*/}" = "dbus-update-activation-environment" ] || ln -s "$f" "$T/nodbus/${f##*/}"
done
for t in cat cp cut date dirname env grep head mkdir mv python3 rm sed sleep stat; do
    [ -e "$T/nodbus/$t" ] || {
        p="$(PATH="$BASE_PATH" command -v "$t")" && ln -s "$p" "$T/nodbus/$t"
    }
done
export PATH="$T/nodbus"
start_launcher
wait_log "start scoot-session.target" || bad "T6: launcher without the dbus tool never reached the session target"
grep -q -F "import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP" "$HARNESS_STATE/calls.log" \
    || bad "T6: no manager-half import without the dbus tool"
env_has WAYLAND_DISPLAY || bad "T6: display missing from the manager without the dbus tool"
ok "T6: without the dbus tool the display still reaches the manager"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T6: launcher exit $RC"
ok "T6: dbus-less session quits cleanly"

echo "---"
echo "$PASS/$TOTAL asserts passed"
[ -n "$KEEP" ] || rm -rf "$ROOT"
