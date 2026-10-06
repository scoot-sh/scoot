#!/bin/sh
# scoot-session-test.sh: a stub harness for `resources/scoot-session`.
#
# Runs the launcher against fake `systemctl --user`, `busctl --user`,
# `flock`, `dbus-update-activation-environment`, `timeout`, `date`,
# `sleep` and `scoot` binaries on PATH, with unit state in files, and
# asserts the session ordering the Asahi bug was about:
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
#   - the session identity rides the same import: `XDG_CURRENT_DESKTOP`
#     defaults to `scoot` and `XDG_SESSION_TYPE` is exported as
#     `wayland` even over a console login's inherited `tty` (the
#     Chrome shape), and both are restored on exit;
#   - without the dbus tool the display still reaches the user manager
#     through `import-environment`;
#   - the session wait blocks in `busctl wait` on the unit's
#     `PropertiesChanged` instead of polling: an idle session asks the
#     manager nothing, a 50-re-exec storm (every `show` failing, the bus
#     staying up, as a real `daemon-reexec` behaves) neither exits the
#     launcher nor costs it a poll, and logout ends the wait at once;
#   - without `busctl` the wait falls back to the 1 s poll, loudly, and
#     the session still works;
#   - a transient resolve failure falls back for a second and re-engages
#     the wait the moment the resolve succeeds;
#   - an unanswered re-ask (manager mid-re-exec) never blocks on a
#     fresh waiter whose wake may already be consumed: it reaps the
#     waiter and polls through the outage, ending the session the
#     moment the manager answers down (T14 pins the stall: a stop that
#     wakes the waiter followed by unanswered re-asks must still exit
#     within ~2 s);
#   - without `timeout(1)` the wait runs bare (no 5-minute stall bound)
#     and the session still works;
#   - the healed login over an already-active graphical target does not
#     re-run its bound units (systemd fires Wants only on the
#     inactive-to-active transition), so they keep the old session's
#     display until logout -- the carried limitation, pinned here rather
#     than fixed: restarting "the session's own" graphical-bound units
#     cannot tell them from another desktop's in the shared-target case
#     (T5's shape), and stopping them all would tear down that session.
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
export REAL_SLEEP

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
# A manager mid-re-exec answers nothing: while $HARNESS_STATE/manager-down
# exists every manager call fails, the way `systemctl --user` exits 1 when
# the manager's socket is gone. The bus (fake busctl below) stays up
# through it, as the real user bus does through a daemon-reexec.
if [ -f "$S/manager-down" ]; then
    case "${1:-}" in
        list-units|show|show-environment|set-environment|unset-environment|import-environment|start|stop|reset-failed)
            exit 1 ;;
    esac
fi
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
                    # systemd activation semantics: Wants fire only on the
                    # inactive-to-active transition. An already-active
                    # graphical target does not re-run its bound units --
                    # which is the carried limitation T10 pins: a heal over
                    # a live foreign graphical target leaves its units on
                    # the old session's display.
                    if [ "$(state_of graphical-session.target)" != "active" ]; then
                        set_state graphical-session.target active
                        # The swayidle shape: WantedBy the graphical target,
                        # gated on the display being in the manager already.
                        if env_has WAYLAND_DISPLAY; then
                            printf 'active' >"$S/swayidle"
                            printf '%s' "$(cat "$S/env-manager")" >"$S/env-at-graphical"
                        else
                            printf 'skipped' >"$S/swayidle"
                        fi
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

# Fake busctl --user: only the two calls scoot-session makes. `call ...
# GetUnit` prints the escaped object path; `wait ... PropertiesChanged`
# blocks until the watched unit's ActiveState file changes, then exits 0
# (like the real wait waking on the signal), or exits 1 after ~15 s so a
# broken launcher fails the suite instead of hanging it. The wait ignores
# $HARNESS_STATE/manager-down: the real user bus survives a manager
# re-exec (measured on systemd 261), so a re-exec must not wake it.
cat >"$FAKES/busctl" <<'EOF'
#!/bin/sh
set -u
S="$HARNESS_STATE"
n=0
if [ -f "$S/calls-seq" ]; then n=$(cat "$S/calls-seq"); fi
n=$((n + 1)); printf '%s' "$n" >"$S/calls-seq"
printf '%s: busctl' "$n" >>"$S/calls.log"
for a in "$@"; do printf ' %s' "$a" >>"$S/calls.log"; done
printf '\n' >>"$S/calls.log"
args=""
for a in "$@"; do
    case "$a" in
        --user|-q|--quiet) continue ;;
        --timeout=*|--destination=*) continue ;;
        *) args="$args $a" ;;
    esac
done
# shellcheck disable=SC2086
set -- $args
op="${1:-}"
if [ "$op" = "call" ]; then
    [ -f "$S/manager-down" ] && exit 1
    # T13's transient resolve failure: GetUnit unanswerable while the
    # flag is present, the way a concurrent heal or a mid-re-exec
    # manager looks to a single resolve round.
    [ -f "$S/bus-call-fail" ] && exit 1
    # ... Manager GetUnit s <unit>: last word is the unit name.
    unit=""
    for a in "$@"; do
        case "$a" in -*) continue ;; esac
        unit="$a"
    done
    esc=$(printf '%s' "$unit" | sed 's/-/_2d/g; s/\./_2e/g')
    printf 'o "/org/freedesktop/systemd1/unit/%s"\n' "$esc"
    exit 0
fi
if [ "$op" = "wait" ]; then
    # wait <service> <object-path> <interface> <signal>
    path="${3:-}"
    name="${path##*/unit/}"
    unit=$(printf '%s' "$name" | sed 's/_2d/-/g; s/_2e/./g')
    before=""
    [ -f "$S/active/$unit" ] && before=$(cat "$S/active/$unit")
    # Baseline captured: stamping wait-ready AFTER it orders the
    # driver's flip strictly past the capture (the driver waits for
    # this file), so no flip can slip between capture and blocking.
    # The calls.log line alone cannot order that: it is written before
    # the capture, in a sibling process the scheduler may stall.
    printf 'ready' >"$S/wait-ready"
    i=0
    while [ "$i" -lt 300 ]; do
        after=""
        [ -f "$S/active/$unit" ] && after=$(cat "$S/active/$unit")
        [ "$after" != "$before" ] && exit 0
        "${REAL_SLEEP:-sleep}" 0.05
        i=$((i + 1))
    done
    exit 1
fi
exit 1
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

# A fake polkit agent: records each run (the display it saw, whether
# the launcher scrubbed the inherited session id) and exits at once,
# so the supervision loop respawns it every 2 s. An instant exit is
# the harshest supervision test: every respawn is a fresh run.
cat >"$FAKES/agent" <<'EOF'
#!/bin/sh
printf 'run DISPLAY=[%s] XDG_SESSION_ID=[%s]\n' "${WAYLAND_DISPLAY-unset}" "${XDG_SESSION_ID-unset}" >>"$HARNESS_STATE/agent-runs.log"
exit 0
EOF
chmod +x "$FAKES/agent"
# A long-lived agent, as the real ones are: records its pid, then waits.
cat >"$FAKES/agent-long" <<AGENT
#!/bin/sh
echo \$\$ >"\$HARNESS_STATE/agent-long.pid"
exec "$REAL_SLEEP" 600
AGENT
chmod +x "$FAKES/agent-long"

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
        "$HARNESS_STATE/env-at-graphical" "$HARNESS_STATE/manager-down" \
        "$HARNESS_STATE/bus-call-fail" "$HARNESS_STATE/wait-ready"
    export ANSWER_AFTER=3 DBUS_FAIL=0
    mksock wayland-99
    export SCOOT_BIN="$SCOOT_FAKE"
}

state_of() { f="$HARNESS_STATE/active/$1"; [ -f "$f" ] && cat "$f" || printf 'inactive'; }
set_state() { printf '%s' "$2" >"$HARNESS_STATE/active/$1"; }
env_has() { grep -q "^$1=" "$HARNESS_STATE/env-manager" 2>/dev/null; }
call_index() { grep -n -F "$1" "$HARNESS_STATE/calls.log" | head -1 | cut -d: -f1; }
show_count() { grep -c -F "show -p ActiveState" "$HARNESS_STATE/calls.log" 2>/dev/null || printf '0'; }

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
wait_stderr() {
    # wait_stderr <fixed-string> : the launcher's stderr says it, up to
    # ~10 s. Notes land after the call that triggers them, so asserting
    # them straight after a wait_log races; wait instead.
    i=0
    while [ "$i" -lt 100 ]; do
        grep -q -F "$1" "$T/stderr.log" 2>/dev/null && return 0
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
wait_blocked() {
    # wait_blocked <tag>: the launcher has subscribed its blocking unit
    # wait AND captured its baseline (the fake stamps $HARNESS_STATE/
    # wait-ready after the capture, in the same process). Flipping the
    # service only past this point orders every flip strictly after the
    # capture, so the wait cannot miss it. Waiting on the calls.log line
    # alone is not enough: it is written before the capture, in a
    # sibling process the scheduler may stall. The poll fallback (T11)
    # needs no gate: a poll cannot miss a flip, it only delays it.
    i=0
    while [ "$i" -lt 100 ]; do
        [ -f "$HARNESS_STATE/wait-ready" ] && return 0
        "$REAL_SLEEP" 0.1
        i=$((i + 1))
    done
    bad "$1: launcher never entered the blocking unit wait"
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
i_scoped="$(call_index 'dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE')"
[ -n "$i_scoped" ] || bad "T1: no scoped session import before the session start"
i_session="$(call_index 'start scoot-session.target')"
[ "$i_session" -gt "$i_scoped" ] || bad "T1: session target started before the display import"
ok "T1: session target starts after the scoped display import"
i_service="$(call_index 'start scoot.service')"
[ -n "$i_service" ] && [ "$i_service" -lt "$i_session" ] || bad "T1: service not started before the session target"
ok "T1: service starts first, session target after the import"
[ "$(cat "$HARNESS_STATE/swayidle")" = "active" ] || bad "T1: ConditionEnvironment probe unit did not start (got $(cat "$HARNESS_STATE/swayidle"))"
grep -q '^WAYLAND_DISPLAY=wayland-100$' "$HARNESS_STATE/env-at-graphical" || bad "T1: display missing from the manager when the graphical target was reached"
ok "T1: ConditionEnvironment unit starts with the display in the manager"
grep -q '^XDG_CURRENT_DESKTOP=scoot$' "$HARNESS_STATE/env-at-graphical" || bad "T1: desktop name missing from the manager when the graphical target was reached"
grep -q '^XDG_SESSION_TYPE=wayland$' "$HARNESS_STATE/env-at-graphical" || bad "T1: session type missing from the manager when the graphical target was reached"
ok "T1: session identity (desktop scoot, type wayland) reaches the manager with the display"
if grep -q -F "start graphical-session.target" "$HARNESS_STATE/calls.log"; then
    bad "T1: launcher starts graphical-session.target directly (RefuseManualStart)"
fi
ok "T1: graphical target reached by dependency only, never started directly"
wait_blocked "T1"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T1: launcher exit $RC, expected 0"
[ "$(state_of scoot.service)" = "inactive" ] && [ "$(state_of scoot-session.target)" = "inactive" ] \
    && [ "$(state_of graphical-session.target)" = "inactive" ] \
    || bad "T1: units leak after quit (service=$(state_of scoot.service) session=$(state_of scoot-session.target) graphical=$(state_of graphical-session.target))"
ok "T1: quit stops service, session and graphical targets (nothing leaks)"
env_has WAYLAND_DISPLAY && bad "T1: display leaks into the manager after quit"
env_has XDG_CURRENT_DESKTOP && bad "T1: desktop name leaks into the manager after quit"
env_has XDG_SESSION_TYPE && bad "T1: session type leaks into the manager after quit"
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
env_has XDG_CURRENT_DESKTOP && bad "T2: desktop name leaks into the manager after the deadline"
env_has XDG_SESSION_TYPE && bad "T2: session type leaks into the manager after the deadline"
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
wait_blocked "T3"
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
grep -q -F "import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE" "$HARNESS_STATE/calls.log" \
    || bad "T6: no manager-half import without the dbus tool"
env_has WAYLAND_DISPLAY || bad "T6: display missing from the manager without the dbus tool"
env_has XDG_CURRENT_DESKTOP || bad "T6: desktop name missing from the manager without the dbus tool"
env_has XDG_SESSION_TYPE || bad "T6: session type missing from the manager without the dbus tool"
ok "T6: without the dbus tool the session identity still reaches the manager"
wait_blocked "T6"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T6: launcher exit $RC"
ok "T6: dbus-less session quits cleanly"

# --- T7: the session wait blocks instead of polling ------------------
new_test 7
start_launcher
wait_blocked "T7"
grep -q -F "scoot_2eservice" "$HARNESS_STATE/calls.log" \
    || bad "T7: blocking wait watches the wrong object (no scoot_2eservice path)"
ok "T7: session wait blocks in busctl wait on scoot.service's PropertiesChanged"
# (No structural assert on subscribe-before-check is possible here: the
# waiter runs in the background, so its log line races the foreground
# re-ask by design. The ordering lives in the launcher -- spawn first,
# ask after, `timeout 300` bounding the residual sliver -- and T9
# proves the wait still ends at once.)
c1="$(show_count)"
"$REAL_SLEEP" 0.6
c2="$(show_count)"
[ "$c1" = "$c2" ] || bad "T7: idle session polled the manager $((c2 - c1)) times in 0.6 s (shows $c1 -> $c2)"
ok "T7: idle session asks the manager nothing while blocked (shows steady at $c1)"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T7: launcher exit $RC"
ok "T7: blocked session quits cleanly"

# --- T8: a re-exec storm neither exits nor costs a poll --------------
new_test 8
start_launcher
wait_blocked "T8"
c1="$(show_count)"
i=0
while [ "$i" -lt 50 ]; do
    : >"$HARNESS_STATE/manager-down"
    "$REAL_SLEEP" 0.02
    rm -f "$HARNESS_STATE/manager-down"
    "$REAL_SLEEP" 0.02
    i=$((i + 1))
done
[ -f "$T/exit-code" ] && bad "T8: launcher exited during the 50-re-exec storm (the #425 shape)"
ok "T8: launcher survives 50 re-execs without reading one as the session ending"
c2="$(show_count)"
[ "$c1" = "$c2" ] || bad "T8: storm cost $((c2 - c1)) manager polls (shows $c1 -> $c2)"
ok "T8: the storm costs zero polls (shows steady at $c1)"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T8: launcher exit $RC after the storm"
ok "T8: post-storm session quits cleanly"

# --- T9: logout ends the blocked wait at once ------------------------
new_test 9
start_launcher
wait_blocked "T9"
t0="$("$REAL_DATE" +%s)"
set_state scoot.service inactive
RC="$(wait_exit)" || bad "T9: launcher never exited after logout"
t1="$("$REAL_DATE" +%s)"
[ "$RC" = "0" ] || bad "T9: launcher exit $RC, expected 0"
dt=$((t1 - t0))
[ "$dt" -le 5 ] || bad "T9: logout took ${dt}s to end the session"
ok "T9: logout ends the session in ${dt}s (exit $RC)"

# --- T10: the carried limitation: a live foreign graphical target ----
# keeps its units' old display past a heal
new_test 10
printf 'WAYLAND_DISPLAY=wayland-99\n' >"$HARNESS_STATE/env-manager"
printf 'WAYLAND_DISPLAY=wayland-99\n' >"$HARNESS_STATE/env-at-graphical"
printf 'active' >"$HARNESS_STATE/swayidle"
set_state graphical-session.target active
set_state scoot.service active
set_state scoot-session.target active
start_launcher
wait_log "start scoot-session.target" || bad "T10: healed login never reached the session target"
[ "$(state_of graphical-session.target)" = "active" ] \
    || bad "T10: heal stopped another session's graphical target"
ok "T10: heal leaves the shared graphical target alone"
grep -q '^WAYLAND_DISPLAY=wayland-100$' "$HARNESS_STATE/env-manager" \
    || bad "T10: healed login did not import its own display"
[ "$(cat "$HARNESS_STATE/swayidle")" = "active" ] \
    || bad "T10: healed login re-ran the graphical-bound probe (got $(cat "$HARNESS_STATE/swayidle"))"
grep -q '^WAYLAND_DISPLAY=wayland-99$' "$HARNESS_STATE/env-at-graphical" \
    || bad "T10: graphical-bound units unexpectedly re-ran with the new display"
ok "T10: graphical-bound units keep the old display until logout (carried limitation, pinned)"
wait_blocked "T10"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T10: launcher exit $RC"
ok "T10: session over a foreign target quits cleanly"

# --- T11: no busctl: the 1 s poll fallback, loudly -------------------
new_test 11
mkdir -p "$T/nobusctl"
for f in "$FAKES"/*; do
    [ "${f##*/}" = "busctl" ] || ln -s "$f" "$T/nobusctl/${f##*/}"
done
for t in cat cp cut date dirname env grep head mkdir mv python3 rm sed sleep stat; do
    [ -e "$T/nobusctl/$t" ] || {
        p="$(PATH="$BASE_PATH" command -v "$t")" && ln -s "$p" "$T/nobusctl/$t"
    }
done
export PATH="$T/nobusctl"
start_launcher
wait_log "start scoot-session.target" || bad "T11: launcher without busctl never reached the session target"
ok "T11: without busctl the session still reaches its target"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T11: launcher exit $RC"
ok "T11: bus-less session quits cleanly"
# After exit the logs are complete: the fallback note must be there, and
# no bus call anywhere in the run.
grep -q -F "1 s poll" "$T/stderr.log" \
    || bad "T11: poll fallback ran silently (no '1 s poll' note on stderr)"
ok "T11: without busctl the session still works, saying it polls"
if grep -q -F "busctl" "$HARNESS_STATE/calls.log"; then bad "T11: launcher called busctl with none on PATH"; fi
ok "T11: fallback calls no bus at all"

# --- T12: no timeout(1): the bare blocking wait still works -----------
new_test 12
mkdir -p "$T/notimeout"
for f in "$FAKES"/*; do
    [ "${f##*/}" = "timeout" ] || ln -s "$f" "$T/notimeout/${f##*/}"
done
for t in cat cp cut date dirname env grep head mkdir mv python3 rm sed sleep stat; do
    [ -e "$T/notimeout/$t" ] || {
        p="$(PATH="$BASE_PATH" command -v "$t")" && ln -s "$p" "$T/notimeout/$t"
    }
done
export PATH="$T/notimeout"
start_launcher
wait_log "start scoot-session.target" || bad "T12: launcher without timeout never reached the session target"
wait_blocked "T12"
ok "T12: without timeout the session still blocks in the wait"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T12: launcher exit $RC"
ok "T12: timeout-less session quits cleanly"

# --- T13: a transient resolve failure costs a second, not the session --
new_test 13
: >"$HARNESS_STATE/bus-call-fail"
start_launcher
wait_log "start scoot-session.target" || bad "T13: launcher never reached the session target"
wait_stderr "1 s poll" || bad "T13: resolve failure did not fall back loudly"
ok "T13: an unresolvable unit wait falls back to the poll, saying so"
rm -f "$HARNESS_STATE/bus-call-fail"
wait_blocked "T13"
wait_stderr "available after all" || bad "T13: recovered resolve did not re-engage loudly"
ok "T13: the wait re-engages the moment the resolve succeeds"
c1="$(show_count)"
"$REAL_SLEEP" 0.6
c2="$(show_count)"
[ "$c1" = "$c2" ] || bad "T13: re-engaged session polled $((c2 - c1)) times in 0.6 s (shows $c1 -> $c2)"
ok "T13: re-engaged session asks nothing while blocked (shows steady at $c1)"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T13: launcher exit $RC"
ok "T13: recovered session quits cleanly"

# --- T14: a wake consumed by an outage still ends the session -------
# The N1 shape: the terminal stop wakes the waiter (its signal
# consumed), the next re-asks land in a manager-down window (rc 2),
# then the manager answers "down". Polling through the outage ends the
# session at once; blocking on the fresh waiter -- whose baseline
# already includes the stop, so nothing will ever wake it -- stalls to
# the wait's timeout. Driver order (down flag first, then the stop)
# guarantees the wake's re-ask lands in the window whatever the
# scheduler does: pre-flip the launcher is blocked in its waiter with
# no round in flight, so on the unfixed code exactly one more `show`
# ever lands (the wake's re-ask) and the gate below never passes.
new_test 14
start_launcher
wait_blocked "T14"
c0="$(show_count)"
: >"$HARNESS_STATE/manager-down"
set_state scoot.service inactive
# The launcher must keep re-asking through the outage (each unanswered
# re-ask is one more `show`): two more within 5 s. The fixed loop
# lands them in milliseconds (its `sleep 1` is instant under the fake);
# the unfixed loop blocks on the fresh waiter after the first and the
# count sticks at c0+1.
i=0
while [ "$i" -lt 50 ]; do
    [ "$(show_count)" -ge "$((c0 + 2))" ] && break
    "$REAL_SLEEP" 0.1
    i=$((i + 1))
done
[ "$(show_count)" -ge "$((c0 + 2))" ] \
    || bad "T14: launcher stopped asking while the manager was down (shows stuck at $(show_count), was $c0)"
ok "T14: unanswered re-asks poll through the outage instead of blocking on a fresh waiter"
# The outage over and the service down, the session must end at once:
# gone within ~2 s, not at the wait's timeout.
t0="$("$REAL_DATE" +%s)"
rm -f "$HARNESS_STATE/manager-down"
i=0
while [ "$i" -lt 20 ]; do
    [ -f "$T/exit-code" ] && break
    "$REAL_SLEEP" 0.1
    i=$((i + 1))
done
[ -f "$T/exit-code" ] || bad "T14: session still hanging past the outage (the N1 stall: fresh waiter, consumed wake)"
RC="$(cat "$T/exit-code")"
t1="$("$REAL_DATE" +%s)"
[ "$RC" = "0" ] || bad "T14: launcher exit $RC, expected 0"
dt=$((t1 - t0))
[ "$dt" -le 2 ] || bad "T14: session took ${dt}s to end past the outage"
ok "T14: session ends in ${dt}s once the manager answers down (exit $RC)"

# --- T15: a console login still exports a Wayland session -------------
# The Chrome shape: `scoot-session` by hand from a VT (or over ssh)
# inherits `XDG_SESSION_TYPE=tty` -- describing the terminal it was run
# from, not the session being started -- and no desktop name. The
# launcher must still export `wayland`/`scoot` to the manager and the
# bus (the scoped import overwrites whatever the login sweep carried),
# and on exit restore the manager's own prior value rather than
# unsetting it. Fail-before: without the forced export the manager
# keeps `tty` and Chrome captures through X11 even on Wayland.
new_test 15
printf 'XDG_SESSION_TYPE=tty\n' >"$HARNESS_STATE/env-manager"
export XDG_SESSION_TYPE=tty
start_launcher
wait_log "start scoot-session.target" || { unset XDG_SESSION_TYPE; bad "T15: console login never reached the session target"; }
unset XDG_SESSION_TYPE
grep -q '^XDG_SESSION_TYPE=wayland$' "$HARNESS_STATE/env-manager" \
    || bad "T15: console login kept the terminal's tty session type (got $(grep '^XDG_SESSION_TYPE=' "$HARNESS_STATE/env-manager" || printf 'nothing'))"
ok "T15: console login exports XDG_SESSION_TYPE=wayland past an inherited tty"
grep -q '^XDG_CURRENT_DESKTOP=scoot$' "$HARNESS_STATE/env-manager" \
    || bad "T15: console login has no desktop name in the manager"
ok "T15: console login defaults the desktop name to scoot"
wait_blocked "T15"
RC="$(end_session)"
[ "$RC" = "0" ] || bad "T15: launcher exit $RC"
grep -q '^XDG_SESSION_TYPE=tty$' "$HARNESS_STATE/env-manager" \
    || bad "T15: prior session type not restored on exit"
env_has XDG_CURRENT_DESKTOP && bad "T15: defaulted desktop name leaks into the manager after quit"
ok "T15: exit restores the prior session type and drops the defaulted desktop name"

# --- T16: a configured agent spawns in the session, scrubbed --------
# `SCOOT_POLKIT_AGENT` names the agent (a bare path, the NixOS module
# renders the store path into the session entry): the launcher starts
# it once the display is known -- a polkit agent registers against
# its own logind session, and only the session scope has one -- with
# the inherited `XDG_SESSION_ID` (the greeter's, or the console's)
# scrubbed, so it can only register against its own scope.
# Fail-before: without the spawn nothing answers prompts; with the id
# inherited the agent registers against the wrong session.
new_test 16
export SCOOT_POLKIT_AGENT="$FAKES/agent"
export XDG_SESSION_ID=greeter-session-id
start_launcher
wait_log "start scoot-session.target" || { unset SCOOT_POLKIT_AGENT XDG_SESSION_ID; bad "T16: agent login never reached the session target"; }
i=0
while [ "$i" -lt 100 ] && [ ! -f "$HARNESS_STATE/agent-runs.log" ]; do "$REAL_SLEEP" 0.1; i=$((i + 1)); done
[ -f "$HARNESS_STATE/agent-runs.log" ] || { unset SCOOT_POLKIT_AGENT XDG_SESSION_ID; bad "T16: configured agent never ran"; }
ok "T16: configured agent spawns once the session is up"
grep -q '^run DISPLAY=\[wayland-100\]' "$HARNESS_STATE/agent-runs.log" \
    || { unset SCOOT_POLKIT_AGENT XDG_SESSION_ID; bad "T16: agent ran before the display was known (got $(head -1 "$HARNESS_STATE/agent-runs.log"))"; }
ok "T16: agent spawns after the display import, seeing the session display"
grep -q 'XDG_SESSION_ID=\[unset\]$' "$HARNESS_STATE/agent-runs.log" \
    || { unset SCOOT_POLKIT_AGENT XDG_SESSION_ID; bad "T16: agent inherited XDG_SESSION_ID (got $(head -1 "$HARNESS_STATE/agent-runs.log"))"; }
ok "T16: agent runs with the inherited session id scrubbed"
wait_blocked "T16"
RC="$(end_session)"
unset SCOOT_POLKIT_AGENT XDG_SESSION_ID
[ "$RC" = "0" ] || bad "T16: launcher exit $RC"
if grep -q -F "scoot-shutdown.target" "$HARNESS_STATE/calls.log"; then
    ok "T16: agent login tears the session down on exit"
else
    bad "T16: agent login never reached the shutdown target"
fi

# --- T17: a dead agent restarts --------------------------------------
# The fake exits at once, so the run count must grow while the
# session stands (the harness fakes `sleep`, so this proves restart,
# not the 2 s backoff -- that is in the loop above, by reading).
# Fail-before: a bare `&` leaves one run and never restarts, failing
# every later prompt silently until re-login.
new_test 17
export SCOOT_POLKIT_AGENT="$FAKES/agent"
start_launcher
wait_log "start scoot-session.target" || { unset SCOOT_POLKIT_AGENT; bad "T17: agent login never reached the session target"; }
i=0
while [ "$i" -lt 100 ] && [ ! -f "$HARNESS_STATE/agent-runs.log" ]; do "$REAL_SLEEP" 0.1; i=$((i + 1)); done
[ -f "$HARNESS_STATE/agent-runs.log" ] || { unset SCOOT_POLKIT_AGENT; bad "T17: configured agent never ran"; }
before="$(wc -l <"$HARNESS_STATE/agent-runs.log")"
"$REAL_SLEEP" 2.5
after="$(wc -l <"$HARNESS_STATE/agent-runs.log")"
[ "$after" -gt "$before" ] || { unset SCOOT_POLKIT_AGENT; bad "T17: dead agent never restarted (runs stayed at $before)"; }
ok "T17: dead agent restarts ($before -> $after runs)"
wait_blocked "T17"
RC="$(end_session)"
unset SCOOT_POLKIT_AGENT
[ "$RC" = "0" ] || bad "T17: launcher exit $RC"

# --- T19: ending the session stops a running agent ------------------
# A real agent lives for the whole session. Stopping the watcher alone
# would orphan it into the scope, still answering prompts for a session
# that is gone (and on the `die` path, for one that never started).
# Fail-before: the agent's pid outlives the launcher.
new_test 19
export SCOOT_POLKIT_AGENT="$FAKES/agent-long"
start_launcher
wait_log "start scoot-session.target" || { unset SCOOT_POLKIT_AGENT; bad "T19: agent login never reached the session target"; }
i=0
while [ "$i" -lt 100 ] && [ ! -s "$HARNESS_STATE/agent-long.pid" ]; do "$REAL_SLEEP" 0.1; i=$((i + 1)); done
[ -s "$HARNESS_STATE/agent-long.pid" ] || { unset SCOOT_POLKIT_AGENT; bad "T19: long-lived agent never ran"; }
agent_long_pid="$(cat "$HARNESS_STATE/agent-long.pid")"
kill -0 "$agent_long_pid" 2>/dev/null || { unset SCOOT_POLKIT_AGENT; bad "T19: long-lived agent is not running"; }
wait_blocked "T19"
RC="$(end_session)"
unset SCOOT_POLKIT_AGENT
[ "$RC" = "0" ] || bad "T19: launcher exit $RC"
i=0
while [ "$i" -lt 30 ] && kill -0 "$agent_long_pid" 2>/dev/null; do "$REAL_SLEEP" 0.1; i=$((i + 1)); done
if kill -0 "$agent_long_pid" 2>/dev/null; then
    kill "$agent_long_pid" 2>/dev/null
    bad "T19: the agent outlived the session (pid $agent_long_pid)"
fi
ok "T19: ending the session stops the running agent"

# --- T18: a missing agent binary is loud, not a wedge ----------------
# A set-but-not-executable path notes once on stderr and the login
# proceeds without prompts (polkit's own no-agent refusal covers
# privileged actions from there). Fail-before: blindly executing it
# loops a failing spawn every 2 s, spamming the greeter log.
new_test 18
export SCOOT_POLKIT_AGENT="$FAKES/no-such-agent"
start_launcher
wait_stderr "SCOOT_POLKIT_AGENT names nothing executable" || { unset SCOOT_POLKIT_AGENT; bad "T18: missing agent binary noted nothing"; }
ok "T18: missing agent binary notes once on stderr"
wait_log "start scoot-session.target" || { unset SCOOT_POLKIT_AGENT; bad "T18: login without an agent never reached the session target"; }
ok "T18: login proceeds without prompts when the agent is missing"
[ ! -f "$HARNESS_STATE/agent-runs.log" ] || { unset SCOOT_POLKIT_AGENT; bad "T18: missing agent binary ran anyway"; }
wait_blocked "T18"
RC="$(end_session)"
unset SCOOT_POLKIT_AGENT
[ "$RC" = "0" ] || bad "T18: launcher exit $RC"

echo "---"
echo "$PASS/$TOTAL asserts passed"
[ -n "$KEEP" ] || rm -rf "$ROOT"
