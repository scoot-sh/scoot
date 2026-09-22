#!/usr/bin/env bash
# Asahi.md Test 4: alternating A/B of the two --tty presentation tiers on real
# hardware -- dumb buffers + pixman (the default) vs GPU scanout via
# DrmCompositor (`--renderer gles`, gpu-scanout build).
#
# Unattended by design. The --tty seat can be held by only one process, and on
# a laptop whose desktop session *is* scoot the only safe way to free it is a
# VT switch -- which takes the operator's terminal with it. So this script
# takes no input and writes every raw number to files:
#
#   Ctrl+Alt+F2, log in, cd to this checkout
#   scripts/tty-tier-bench.sh
#
# **Stay on that VT until it finishes** (~7 minutes at the defaults). This is
# not a comfort preference: a `--tty` compositor on an inactive VT is *paused*
# by logind -- it holds no DRM master, renders nothing and flips nothing -- so
# switching back to your session mid-run would leave the benchmark measuring a
# compositor that is doing no work, and the jiffy counts would look
# spectacular for exactly the wrong reason. Every round therefore checks its
# own log for a pause and marks the row `paused` rather than reporting it.
#
# It records rather than narrates (CLAUDE.md): per-round logs, raw jiffy
# counters, raw microwatt samples, screenshots, and a TSV. Analysis happens
# afterwards, off this machine's critical path, so nothing here needs jq,
# python or ImageMagick -- bash, coreutils, the two scoot builds and foot.
#
# Overrides: SCOOT_DUMB, SCOOT_GPU, SCOOTCTL (binaries), OUT (output dir),
# ROUNDS, IDLE_SECS, MOVE_SECS/MOVE_GAP, WIDTH_SECS/WIDTH_GAP, BACKEND.
set -uo pipefail

# Same lesson as nested-resize-repro.sh: default to the tree you invoked from
# rather than an absolute shared path, which has handed agents someone else's
# binary before. These two must come from the *same* commit or the A/B is
# meaningless -- the script prints both versions and mtimes so the record shows
# whether they did.
SCOOT_DUMB=${SCOOT_DUMB:-result-scoot/bin/scoot}
SCOOT_GPU=${SCOOT_GPU:-result-scoot-gpu/bin/scoot}
# The client is a separate package and the compositor packages deliberately do
# not ship it (`packages.scoot` builds `-p scoot` only -- see
# docs/backlog/resolved/scoot-package-ships-scootctl-done.md), so it cannot be
# assumed to sit next to the binary under test. Resolved and checked *before*
# any seat is taken: discovering it missing mid-round would waste a VT trip.
SCOOTCTL=${SCOOTCTL:-}
OUT=${OUT:-/tmp/scoot-tier-bench}
ROUNDS=${ROUNDS:-4}
IDLE_SECS=${IDLE_SECS:-10}
# Damage is driven for a fixed wall-clock window at a fixed rate, not for a
# fixed event count -- see scene 2. 60/s of cursor damage and 20/s of
# full-output relayout both sit under a 60Hz panel's refresh, so each event
# gets its own frame instead of being coalesced away.
MOVE_SECS=${MOVE_SECS:-15}
MOVE_GAP=${MOVE_GAP:-0.0155}
WIDTH_SECS=${WIDTH_SECS:-10}
WIDTH_GAP=${WIDTH_GAP:-0.05}
MOVES_DONE=0
WIDTHS_DONE=0
# `--headless` exists here only to rehearse the harness itself inside a
# running session: it has no CRTC, so it cannot answer Test 4 (the scanout
# tier is tty-only by design -- render/gles.rs:15-24). It does prove the
# measurement plumbing and that EGL/GLES comes up on this GPU at all.
BACKEND=${BACKEND:---tty}
HEADLESS_SIZE=(--width 2560 --height 1600)

for b in "$SCOOT_DUMB" "$SCOOT_GPU"; do
    if [ ! -x "$b" ]; then
        echo "no scoot binary at $b -- nix build .#scoot -o result-scoot and" >&2
        echo "nix build .#scoot-gpu -o result-scoot-gpu, or set SCOOT_DUMB/SCOOT_GPU" >&2
        exit 1
    fi
done
if [ -z "$SCOOTCTL" ]; then
    for c in result-scootctl/bin/scootctl \
             "$(dirname "$SCOOT_DUMB")/scootctl" \
             "$(dirname "$SCOOT_GPU")/scootctl" \
             "$(command -v scootctl 2>/dev/null)"; do
        if [ -n "$c" ] && [ -x "$c" ]; then SCOOTCTL=$c; break; fi
    done
fi
if [ -z "$SCOOTCTL" ] || ! "$SCOOTCTL" --version >/dev/null 2>&1; then
    echo "no usable scootctl -- nix build .#scootctl -o result-scootctl, or set SCOOTCTL" >&2
    exit 1
fi
command -v foot >/dev/null || { echo "foot is not on PATH -- it is the test client" >&2; exit 1; }
[ -n "${XDG_RUNTIME_DIR:-}" ] || { echo "no XDG_RUNTIME_DIR -- log in on a VT, do not su" >&2; exit 1; }

mkdir -p "$OUT"
SUMMARY="$OUT/summary.tsv"
ENVLOG="$OUT/environment.txt"

# --- the record's cache key (CLAUDE.md: evidence is keyed to a tree state) ---
{
    echo "date: $(date -Is)"
    echo "host: $(uname -srm)  $(tr -d '\0' < /proc/device-tree/compatible 2>/dev/null)"
    echo "git: $(git -C "$(dirname "$0")/.." rev-parse HEAD 2>/dev/null) $(git -C "$(dirname "$0")/.." status --porcelain 2>/dev/null | head -5)"
    echo "ctl:  $SCOOTCTL ($("$SCOOTCTL" --version 2>&1))"
    echo "dumb: $SCOOT_DUMB -> $(readlink -f "$SCOOT_DUMB") ($("$SCOOT_DUMB" --version 2>&1))"
    echo "gpu:  $SCOOT_GPU -> $(readlink -f "$SCOOT_GPU") ($("$SCOOT_GPU" --version 2>&1))"
    echo "gbm linkage: dumb=$(ldd "$SCOOT_DUMB" 2>/dev/null | grep -c gbm) gpu=$(ldd "$SCOOT_GPU" 2>/dev/null | grep -c gbm)"
    echo "backend: $BACKEND"
    echo "vt: $(fgconsole 2>/dev/null || echo '?')  session: ${XDG_SESSION_ID:-?}"
    echo "seat holders (--tty scoot processes, the live session included):"
    pgrep -af -- '--tty' | sed 's/^/  /'
    echo "drm:"
    ls -l /dev/dri/ | sed 's/^/  /'
    for s in /sys/class/drm/card*-*/status; do echo "  $s: $(cat "$s")"; done
    echo "power: ac_online=$(cat /sys/class/power_supply/macsmc-ac/online 2>/dev/null) battery=$(cat /sys/class/power_supply/macsmc-battery/status 2>/dev/null)"
    echo "cpus: $(nproc)"
} > "$ENVLOG"
cat "$ENVLOG"

printf 'round\ttier\tcame_up\tpaused\tconnector\tscanout\tidle_jiffies\tidle_secs\tidle_uW_mean\tmove_jiffies\tmove_events\tmove_ms\tmove_uW_mean\twidth_jiffies\twidth_events\twidth_ms\twidth_uW_mean\trss_kB\n' > "$SUMMARY"

PID=
cleanup() {
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill -TERM "$PID" 2>/dev/null
        for _ in $(seq 40); do kill -0 "$PID" 2>/dev/null || break; sleep 0.1; done
        kill -KILL "$PID" 2>/dev/null
    fi
}
trap 'cleanup; exit 130' INT TERM
trap cleanup EXIT

# utime+stime for the compositor, in jiffies -- fields 14 and 15 of
# /proc/<pid>/stat. Read via the field list *after* the comm field, because a
# process name can contain spaces and shift every later column.
cpu_jiffies() {
    local p=$1
    [ -r "/proc/$p/stat" ] || { echo ""; return; }
    awk '{ s=$0; sub(/^[0-9]+ \(.*\) /, "", s); split(s, f, " "); print f[12] + f[13] }' "/proc/$p/stat"
}
rss_kb() { awk '/^VmRSS:/ {print $2}' "/proc/$1/status" 2>/dev/null; }
# Mean of a file of `power_now` samples, in microwatts. The sysfs value is
# signed -- negative while discharging -- so take the magnitude; a machine on
# AC reads ~0 and the mean is then meaningless, which `environment.txt`'s
# `ac_online` line is there to disclose.
power_mean() {
    awk '{ s += ($1 < 0 ? -$1 : $1); n++ } END { if (n) printf "%.0f", s/n; else print "-" }' "$1" 2>/dev/null
}
now_ms() { echo $(( $(date +%s%N) / 1000000 )); }

run_round() {
    local round=$1 tier=$2 bin=$3
    shift 3
    local extra=("$@")
    local tag="r${round}-${tier}"
    local log="$OUT/$tag.log"
    # A unix socket path is capped at 107 bytes and $OUT can be anywhere, so
    # the socket lives in the runtime dir under a short name while every
    # artefact stays in $OUT. Rehearsing this harness is what found it.
    local sock="$XDG_RUNTIME_DIR/stb-$$-$tag.sock"
    local ctl=$SCOOTCTL

    rm -f "$sock"
    echo "=== round $round tier $tier: $bin ${extra[*]}"
    # `env -u` so a stray SCOOT_SOCKET cannot aim this at the live session --
    # exactly the trap that cost Asahi.md's Test 1 its first attempt.
    local backend_args=("$BACKEND")
    [ "$BACKEND" = --headless ] && backend_args+=("${HEADLESS_SIZE[@]}")
    env -u SCOOT_SOCKET "$bin" "${backend_args[@]}" --socket "$sock" "${extra[@]}" -- foot \
        > "$log" 2>&1 &
    PID=$!

    local up=no
    for _ in $(seq 100); do
        if [ -S "$sock" ] && SCOOT_SOCKET="$sock" "$ctl" version >/dev/null 2>&1; then up=yes; break; fi
        kill -0 "$PID" 2>/dev/null || break
        sleep 0.2
    done

    if [ "$up" != yes ]; then
        # The most valuable possible result for the scanout tier, and the one
        # thing that must not be misattributed: a busy seat looks nothing like
        # a GPU failure but fails at the same call. Say which it was.
        #
        # Classify on the binary's own last line -- its startup errors are
        # one-line messages on stderr -- and never on a grep over the whole
        # log. "seat" occurs in every log as a tracing span name
        # (`input_seat{name="scoot"}`), which made the first version of this
        # script report a socket-path-too-long failure as a seat conflict.
        # Misattributing a failure to the mechanism under test is the exact
        # trap CLAUDE.md records from the EPERM/seat-busy incident.
        local last why
        last=$(grep -aE '^scoot: |^error|panicked' "$log" | tail -1)
        [ -n "$last" ] || last=$(tail -1 "$log")
        why="startup failed: ${last:-see $tag.log}"
        case "$last" in
            *libseat*|*"Permission denied"*|*busy*|*"seat "*)
                why="startup failed, SEAT not GPU: $last -- another session is holding the seat. Launch this from the VT you are sitting on." ;;
            *"could not load"*)
                # An absent library is this box's setup, not this GPU's
                # answer, and conflating the two would report a missing
                # libEGL as "scanout does not work on Apple Silicon". The
                # compositor's own message already names the pixman fallback.
                why="startup failed on a MISSING LIBRARY, not the GPU: $last" ;;
            *gbm*|*GBM*|*egl*|*EGL*|*gles*|*scanout*|*drm*|*DRM*)
                why="startup failed inside the GPU/DRM path: $last -- THIS IS THE TEST 4 ANSWER, keep $tag.log" ;;
        esac
        echo "  !! $why"
            printf '%s\t%s\tno\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\n' "$round" "$tier" >> "$SUMMARY"
        cleanup; PID=
        sleep 1
        return 0
    fi

    export SCOOT_SOCKET="$sock"
    # Strip ANSI first. The compositor used to colour its output even when
    # stdout was a file, so `scanout="gpu"` was really
    # `scanout\e[0m\e[2m=\e[0m"gpu"` in the bytes and the naive pattern below
    # matched nothing -- which reported the *most important field in this
    # whole benchmark* as absent on a run where the tier had in fact come up.
    # That is fixed at the source (compositor/mod.rs `init_logging`), and
    # stripping stays here anyway: these logs are also read back from older
    # runs and from any build predating that fix.
    local connector scanout plain
    plain=$(sed -e 's/\x1b\[[0-9;]*m//g' "$log")
    connector=$(printf '%s\n' "$plain" | grep -oE 'connector=[^ ]+' | head -1 | cut -d= -f2)
    scanout=$(printf '%s\n' "$plain" | grep -oE 'scanout="[a-z]+"' | head -1 | cut -d'"' -f2)
    echo "  up: connector=${connector:-?} scanout=${scanout:-none}"
    "$ctl" outputs > "$OUT/$tag.outputs" 2>&1

    # Second window, so the layout scene has something to rearrange.
    "$ctl" action spawn foot >/dev/null 2>&1
    sleep 1.5
    "$ctl" wait-idle --quiet-ms 500 --timeout-ms 8000 >/dev/null 2>&1
    "$ctl" windows > "$OUT/$tag.windows" 2>&1

    # Correctness capture, taken HERE and not at the end of the round: two
    # windows freshly mapped at their default widths with the pointer parked
    # at a fixed spot is a scene both tiers reach identically, so the only
    # thing left between the two PNGs is how each renderer drew it.
    #
    # The first version of this script captured after the damage scenes
    # instead, and the comparison was worthless: `cycle-column-width` had run
    # a different number of times on each tier (156 vs 176 in the 2026-09-21
    # run, since each tier gets through a different count in a fixed window),
    # so the captures showed different column layouts and different cursor
    # positions. Within a single tier, captures from different rounds differed
    # by AE 13853-46852 -- swamping any renderer difference, which for
    # 2560x1600 would be about 4016 if every pixel differed by one
    # least-significant bit. The end-of-round capture is kept too, as a record
    # of where each round finished; it is `-end` and is not the comparison.
    "$ctl" pointer move 1280 800 >/dev/null 2>&1
    "$ctl" wait-idle --quiet-ms 500 --timeout-ms 8000 >/dev/null 2>&1
    "$ctl" screenshot --out "$OUT/$tag-pinned.png" >/dev/null 2>&1

    # --- scene 1: idle. Nothing moving; sample power across the same window.
    local j0 j1 idle_j
    j0=$(cpu_jiffies "$PID")
    : > "$OUT/$tag.power"
    for _ in $(seq "$IDLE_SECS"); do
        cat /sys/class/power_supply/macsmc-battery/power_now 2>/dev/null >> "$OUT/$tag.power"
        sleep 1
    done
    j1=$(cpu_jiffies "$PID")
    idle_j=$(( j1 - j0 ))
    local uw
    uw=$(power_mean "$OUT/$tag.power")

    # --- scene 2: cursor damage, PACED. The dev VM's comparable number
    # (06-gpu-pipeline.md "Benchmark") was 300 unpaced IPC pointer moves, and
    # copying that literally here measures the wrong thing: an injection round
    # trip costs ~0.8ms on this machine against the VM's ~11ms, so an unpaced
    # burst arrives ~20x faster than the panel refreshes and the compositor
    # correctly coalesces it into a handful of frames. Rehearsing the harness
    # is what surfaced it (3000 moves -> 16 jiffies, i.e. almost no frames).
    #
    # So drive damage at a fixed rate below the refresh rate instead, for a
    # fixed wall-clock window: each event then gets its own frame on both
    # tiers, and the jiffy counts compare CPU spent presenting the *same
    # number of frames* rather than the same number of coalesced bursts.
    # Power is sampled *inside* the damage loops too, roughly once a second.
    # The first run of this script sampled it at idle only, which answered the
    # least interesting version of the question: both tiers sleep completely
    # when nothing moves, so idle power is identical by construction. What the
    # backlog entry actually wants to know is whether the GPU tier trades CPU
    # wakeups for GPU draw *while presenting*, and that needs a sample taken
    # while it is presenting.
    local t0 t1 move_j move_ms x y n
    : > "$OUT/$tag.power-move"
    j0=$(cpu_jiffies "$PID"); t0=$(now_ms); n=0
    while [ "$(( $(now_ms) - t0 ))" -lt $(( MOVE_SECS * 1000 )) ]; do
        n=$(( n + 1 ))
        x=$(( 200 + (n * 7) % 900 )); y=$(( 150 + (n * 11) % 600 ))
        "$ctl" pointer move "$x" "$y" >/dev/null 2>&1
        [ $(( n % 60 )) -eq 0 ] &&
            cat /sys/class/power_supply/macsmc-battery/power_now 2>/dev/null >> "$OUT/$tag.power-move"
        sleep "$MOVE_GAP"
    done
    t1=$(now_ms); j1=$(cpu_jiffies "$PID")
    move_j=$(( j1 - j0 )); move_ms=$(( t1 - t0 )); MOVES_DONE=$n

    # --- scene 3: full-output damage, paced the same way.
    # cycle-column-width relayouts and redraws the whole output --
    # compositor-driven and deterministic, unlike a client animating itself,
    # which would measure the client. This is the scene where scanout should
    # show its advantage, since a full-output frame is the read-back and the
    # dumb-buffer memcpy it deletes.
    local width_j width_ms
    : > "$OUT/$tag.power-width"
    j0=$(cpu_jiffies "$PID"); t0=$(now_ms); n=0
    while [ "$(( $(now_ms) - t0 ))" -lt $(( WIDTH_SECS * 1000 )) ]; do
        n=$(( n + 1 ))
        "$ctl" action cycle-column-width >/dev/null 2>&1
        [ $(( n % 20 )) -eq 0 ] &&
            cat /sys/class/power_supply/macsmc-battery/power_now 2>/dev/null >> "$OUT/$tag.power-width"
        sleep "$WIDTH_GAP"
    done
    t1=$(now_ms); j1=$(cpu_jiffies "$PID")
    width_j=$(( j1 - j0 )); width_ms=$(( t1 - t0 )); WIDTHS_DONE=$n

    local rss
    rss=$(rss_kb "$PID")

    # Did this round actually run on an active VT? logind pauses a --tty
    # session the moment another VT takes the seat, and a paused compositor
    # holds no DRM master, renders nothing and flips nothing -- so its jiffy
    # counts would look excellent for exactly the wrong reason. The log line
    # is `session paused; drm master released` (tty/mod.rs:1217). A row this
    # matches is not a slower number, it is not a number at all.
    local paused=no
    if grep -q 'session paused' "$log"; then
        paused=yes
        echo "  !! VT SWITCHED AWAY DURING THIS ROUND -- the session was paused, so its"
        echo "     numbers measure a compositor doing no work. Re-run this round without"
        echo "     leaving the VT."
    fi

    # Correctness evidence: the frame the tier actually presented, read back
    # through the capture path the tier owns.
    "$ctl" wait-idle --quiet-ms 500 --timeout-ms 8000 >/dev/null 2>&1
    "$ctl" screenshot --out "$OUT/$tag-end.png" >/dev/null 2>&1

    local uw_move uw_width
    uw_move=$(power_mean "$OUT/$tag.power-move")
    uw_width=$(power_mean "$OUT/$tag.power-width")

    printf '%s\t%s\tyes\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$round" "$tier" "$paused" "${connector:-?}" "${scanout:-none}" \
        "$idle_j" "$IDLE_SECS" "$uw" \
        "$move_j" "$MOVES_DONE" "$move_ms" "${uw_move:--}" \
        "$width_j" "$WIDTHS_DONE" "$width_ms" "${uw_width:--}" "${rss:--}" >> "$SUMMARY"
    echo "  idle=${idle_j}j/${IDLE_SECS}s power=${uw}uW" \
         "moves=${move_j}j/${MOVES_DONE}ev/${move_ms}ms/${uw_move}uW" \
         "widths=${width_j}j/${WIDTHS_DONE}ev/${width_ms}ms/${uw_width}uW rss=${rss}kB"

    "$ctl" action quit >/dev/null 2>&1
    for _ in $(seq 50); do kill -0 "$PID" 2>/dev/null || break; sleep 0.1; done
    cleanup; PID=
    unset SCOOT_SOCKET
    sleep 2   # let the seat and the GPU settle before the next tier takes them
}

# Alternate the tiers. Never all of A then all of B: a single ordering on this
# project once read as a 70% regression that was pure noise.
for r in $(seq "$ROUNDS"); do
    run_round "$r" dumb "$SCOOT_DUMB"
    run_round "$r" gpu  "$SCOOT_GPU" --renderer gles
done

echo
echo "=== $SUMMARY"
cat "$SUMMARY"
echo
echo "done. Everything raw is in $OUT (logs, power samples, screenshots, outputs/windows dumps)."
