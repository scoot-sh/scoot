#!/usr/bin/env bash
# A/B resource usage of scoot against niri on identical nested workloads
# (docs/benchmarks.md has the method, the results and every caveat).
#
# Both compositors run *nested*, one at a time, inside the same host: cage on
# wlroots' headless backend with its pixman renderer (HOST_RENDERER below),
# the host `smoke-test.sh`'s `--nested` run uses. On the dev VM that is the only fair
# arrangement there is: niri renders only through GLES and refuses a software
# renderer on `--tty`, and the VM's virtio-gpu has no 3D. `cage -d` asks
# clients not to draw their own decorations, which matters: without it niri's
# winit window draws a frame and its output shrinks to 1592x968 while
# scoot's stays 1600x1000.
#
# Variants, alternated round by round (the order rotates each round):
#   scoot-pixman  scoot --nested (default renderer)
#   scoot-gles    scoot --nested --renderer gles (llvmpipe on the VM)
#   niri-off      niri, animations off (scripts/niri-ab/niri-anim-off.kdl)
#   niri-on       niri, its default animations (same file minus that line)
#
# Per variant session, in this order: startup (exec -> IPC answering), memory
# with an empty session, three `foot`s, memory, then these scenes, each
# measured on the nested compositor's process only (memory is sampled again
# after the two screenshot scenes and at the end):
#   idle       IDLE_SECS with nothing happening
#   pointer    absolute pointer motion between two points in two different
#              windows at MOTION_HZ for MOTION_SECS, injected into the *host*
#              by one persistent zwlr_virtual_pointer_v1 (scripts/niri-ab/vptr)
#              -- the same input path for both compositors
#   relayout   focus-column / move-column cycling at RELAYOUT_HZ for
#              RELAYOUT_SECS, through each compositor's own IPC client
#              (`scootctl action focus-column left` vs `niri msg action
#              focus-column-left`): an unavoidable asymmetry, the clients
#              are different programs (their CPU is not counted)
#   shot-ipc   SHOTS captures through each compositor's own screenshot path
#   shot-grim  SHOTS captures by `grim` against the nested session
#              (ext-image-copy-capture on scoot, wlr-screencopy on niri)
#   animate    a fourth foot printing a line every 16 ms for ANIM_SECS
#
# Every number is raw and per round, in $OUT/results.tsv, $OUT/memory.tsv,
# $OUT/startup.tsv and $OUT/shots.tsv; scripts/niri-ab/summarize.sh reduces
# them. CPU is summed over every thread (/proc/PID/task/*/schedstat, ns) and
# cross-checked against /proc/PID/stat's utime+stime, which also keeps the
# time of threads that exited mid-scene. "Wakeups" is the summed schedstat
# run count: how many times any thread of the compositor was put on a CPU.
#
# DIAG=1 runs the same thing with WAYLAND_DEBUG=server on the host, and from
# the host's protocol log records how many frames (commits of the nested
# compositor's toplevel) each scene produced and the time from exec to the
# first frame. That log costs the host CPU and perturbs pacing, which is why
# it is a separate pass: CPU numbers from a DIAG run are not results.
#
# Needs: cage, wlr-randr, foot, grim on PATH; niri (NIRI=), scoot and
# scootctl from the same build (SCOOT=, SCOOTCTL=), and the nab-vptr helper
# (VPTR=, built from scripts/niri-ab/vptr with `cargo build
# --release`). Nothing here needs a seat or a GPU.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCOOT=${SCOOT:-${CARGO_TARGET_DIR:-$HERE/../target}/release/scoot}
SCOOTCTL=${SCOOTCTL:-$(dirname "$SCOOT")/scootctl}
NIRI=${NIRI:-$(command -v niri || true)}
VPTR=${VPTR:-}
NIRI_CONF=${NIRI_CONF:-$HERE/niri-ab/niri-anim-off.kdl}
OUT=${OUT:-/tmp/niri-ab}
ROUNDS=${ROUNDS:-3}
VARIANTS=${VARIANTS:-"scoot-pixman scoot-gles niri-off niri-on"}
WIDTH=${WIDTH:-1600}
HEIGHT=${HEIGHT:-1000}
IDLE_SECS=${IDLE_SECS:-20}
MOTION_HZ=${MOTION_HZ:-120}
MOTION_SECS=${MOTION_SECS:-10}
RELAYOUT_HZ=${RELAYOUT_HZ:-20}
RELAYOUT_SECS=${RELAYOUT_SECS:-10}
SHOTS=${SHOTS:-10}
SHOT_GAP=${SHOT_GAP:-0.2}
ANIM_SECS=${ANIM_SECS:-10}
DIAG=${DIAG:-0}
# A session that runs past this is killed, noted in notes.log and the run
# moves on, so a wedged compositor fails the run loudly instead of hanging
# it. (A nested niri under a host that could not build its output never
# answers IPC, and the scenes' own clients are deliberately not wrapped in
# `timeout`: an extra process per event would change what they measure.)
SESSION_TIMEOUT=${SESSION_TIMEOUT:-300}
# The host's renderer. pixman (the default) is what the dev VM results use;
# on a machine with a real GPU, gles2 gives the host linux-dmabuf, which is
# what lets niri's winit backend (and a gpu-scanout scoot's dma-buf
# presenter) use that GPU rather than fall back to software.
HOST_RENDERER=${HOST_RENDERER:-pixman}

die() { echo "niri-ab: $*" >&2; exit 1; }
for b in "$SCOOT" "$SCOOTCTL" "$NIRI" "$VPTR"; do
    [ -n "$b" ] && [ -x "$b" ] || die "missing binary ('$b'): set SCOOT, SCOOTCTL, NIRI and VPTR"
done
for t in cage wlr-randr foot grim; do command -v "$t" >/dev/null || die "$t is not on PATH"; done
[ -n "${XDG_RUNTIME_DIR:-}" ] || die "XDG_RUNTIME_DIR is unset"
[ -e "$OUT/results.tsv" ] && [ "${OVERWRITE:-0}" != 1 ] && die "$OUT already holds a run (OVERWRITE=1 replaces it)"
mkdir -p "$OUT"
rm -f "$OUT"/*.tsv
unset NIRI_SOCKET SCOOT_SOCKET WAYLAND_DEBUG

# Both niri configs are read from $OUT, never from the checkout: niri watches
# its config file, and a file on the dev VM's 9p mount of this checkout cost
# it ~55 extra wakeups/s at idle (measured: 1153 vs 62 over 20 s), which is
# the harness, not niri. The animations-on config is the animations-off one
# minus its one `animations { off; }` line, so the two cannot drift apart.
NIRI_CONF_OFF="$OUT/niri-anim-off.kdl"
NIRI_CONF_ON="$OUT/niri-anim-on.kdl"
cp "$NIRI_CONF" "$NIRI_CONF_OFF"
grep -v '^animations { off; }$' "$NIRI_CONF" >"$NIRI_CONF_ON"
cmp -s "$NIRI_CONF_OFF" "$NIRI_CONF_ON" && die "$NIRI_CONF has no 'animations { off; }' line to remove"
: >"$OUT/foot.ini"
# scoot's `spawn` splits its command on whitespace and runs no shell, so
# anything needing a shell is a script file, spawned the same way on both.
printf '#!/bin/sh\necho "$WAYLAND_DISPLAY" >"$1"\n' >"$OUT/display.sh"
printf '#!/bin/sh\ni=0\nwhile :; do i=$((i + 1)); echo "nab-anim $i"; sleep 0.016; done\n' >"$OUT/anim.sh"

{
    echo "date_utc	$(date -u +%FT%TZ)"
    echo "scoot	$SCOOT	$("$SCOOT" --version)	sha256=$(sha256sum "$SCOOT" | cut -d' ' -f1)	bytes=$(stat -c %s "$SCOOT")"
    echo "scootctl	$SCOOTCTL	$("$SCOOTCTL" --version)"
    echo "niri	$(readlink -f "$NIRI")	$("$NIRI" --version)	bytes=$(stat -c %s "$(readlink -f "$NIRI")")"
    echo "cage	$(readlink -f "$(command -v cage)")"
    echo "foot	$(readlink -f "$(command -v foot)")	$(foot --version)"
    echo "grim	$(readlink -f "$(command -v grim)")"
    echo "kernel	$(uname -r)	cpus=$(nproc)"
    echo "params	host_renderer=$HOST_RENDERER rounds=$ROUNDS size=${WIDTH}x$HEIGHT idle=$IDLE_SECS motion=${MOTION_HZ}Hz/${MOTION_SECS}s relayout=${RELAYOUT_HZ}Hz/${RELAYOUT_SECS}s shots=$SHOTS anim=${ANIM_SECS}s diag=$DIAG"
} >"$OUT/versions.tsv"
cat "$OUT/versions.tsv"

printf 'round\tvariant\tscene\twall_s\tevents\tcpu_ns\tcpu_jiffies\twakeups\tctxsw\tthreads\tcage_cpu_ns\tframes\n' >"$OUT/results.tsv"
printf 'round\tvariant\tpoint\trss_kb\tpss_kb\tpss_anon_kb\tpss_file_kb\tthreads\n' >"$OUT/memory.tsv"
printf 'round\tvariant\tipc_ready_ms\tfirst_frame_ms\n' >"$OUT/startup.tsv"
printf 'round\tvariant\tmethod\ti\twall_ms\tbytes\n' >"$OUT/shots.tsv"

now_ns() { date +%s%N; }

# Summed over every live thread: on-CPU ns, run count, context switches.
task_sums() {
    awk 'FILENAME ~ /schedstat$/ { cpu += $1; runs += $3; n++; next }
         /^(non)?voluntary_ctxt_switches:/ { cs += $2 }
         END { printf "%d %d %d %d\n", cpu, runs, cs, n }' \
        /proc/"$1"/task/*/schedstat /proc/"$1"/task/*/status 2>/dev/null
}
# utime+stime in clock ticks: includes threads that have since exited.
jiffies() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
cpu_ns_only() { local s; s=$(task_sums "$1"); echo "${s%% *}"; }

# Waits until the compositor has used under 2 ms of CPU in each of two
# consecutive 0.5 s windows, or 20 s pass (then says so in the log).
settle() {
    local pid=$1 prev cur quiet=0 i
    prev=$(cpu_ns_only "$pid")
    for i in $(seq 1 40); do
        sleep 0.5
        cur=$(cpu_ns_only "$pid")
        if [ $((cur - prev)) -lt 2000000 ]; then quiet=$((quiet + 1)); else quiet=0; fi
        prev=$cur
        [ "$quiet" -ge 2 ] && return 0
    done
    echo "  (not idle after 20 s)" | tee -a "$OUT/notes.log"
}

memory() { # round variant point pid
    local r
    r=$(awk '/^Rss:/{r=$2} /^Pss:/{p=$2} /^Pss_Anon:/{a=$2} /^Pss_File:/{f=$2} END{print r"\t"p"\t"a"\t"f}' "/proc/$4/smaps_rollup")
    printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$r" "$(ls /proc/"$4"/task | wc -l)" >>"$OUT/memory.tsv"
}

# Host-log frame count for the nested toplevel (DIAG only).
frames() {
    [ "$DIAG" = 1 ] && [ -n "$SURF" ] || { echo "-"; return; }
    grep -c "wl_surface#$SURF\.commit()" "$HOST_LOG"
}

SCENE_T0=0
scene_begin() {
    SCENE_T0=$(now_ns)
    read -r S_CPU S_RUNS S_CS S_N <<<"$(task_sums "$PID")"
    S_J=$(jiffies "$PID"); S_CAGE=$(cpu_ns_only "$CAGE_PID"); S_F=$(frames)
}
scene_end() { # round variant scene events
    local t1 e_cpu e_runs e_cs e_n e_j e_cage e_f fr
    t1=$(now_ns)
    read -r e_cpu e_runs e_cs e_n <<<"$(task_sums "$PID")"
    e_j=$(jiffies "$PID"); e_cage=$(cpu_ns_only "$CAGE_PID"); e_f=$(frames)
    if [ "$e_f" = "-" ]; then fr="-"; else fr=$((e_f - S_F)); fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" \
        "$(awk -v a="$SCENE_T0" -v b="$t1" 'BEGIN{printf "%.3f", (b-a)/1e9}')" "$4" \
        $((e_cpu - S_CPU)) $((e_j - S_J)) $((e_runs - S_RUNS)) $((e_cs - S_CS)) "$S_N/$e_n" \
        $((e_cage - S_CAGE)) "$fr" >>"$OUT/results.tsv"
    printf '  %-9s events=%-5s cpu=%6.1f ms wakeups=%-6s frames=%s\n' "$3" "$4" \
        "$(awk -v n=$((e_cpu - S_CPU)) 'BEGIN{print n/1e6}')" $((e_runs - S_RUNS)) "$fr"
}

# Runs "$@" at HZ for SECS against a deadline that never bursts to catch up.
# Echoes how many times it ran.
paced() {
    local hz=$1 secs=$2; shift 2
    local period_ns start next now n=0
    period_ns=$(awk -v h="$hz" 'BEGIN{printf "%d", 1e9/h}')
    start=$(now_ns); next=$start
    while [ $(( $(now_ns) - start )) -lt $((secs * 1000000000)) ]; do
        "$@" "$n" >/dev/null 2>&1
        n=$((n + 1)); next=$((next + period_ns)); now=$(now_ns)
        if [ "$next" -gt "$now" ]; then
            sleep "$(awk -v d=$((next - now)) 'BEGIN{printf "%.6f", d/1e9}')"
        else
            next=$now
        fi
    done
    echo "$n"
}

png_done() { [ -s "$1" ] && tail -c 12 "$1" | grep -q IEND; }

run_session() { # round variant
    local round=$1 v=$2 dir="$OUT/r$1-$2" cmd kind
    mkdir -p "$dir/cfg"
    HOST_LOG="$dir/host.log"; SURF=""
    case $v in
        scoot-pixman) kind=scoot; cmd="$SCOOT --nested --width $WIDTH --height $HEIGHT --socket $dir/ipc.sock" ;;
        scoot-gles) kind=scoot; cmd="$SCOOT --nested --renderer gles --width $WIDTH --height $HEIGHT --socket $dir/ipc.sock" ;;
        niri-off) kind=niri; cmd="$NIRI -c $NIRI_CONF_OFF" ;;
        niri-on) kind=niri; cmd="$NIRI -c $NIRI_CONF_ON" ;;
        *) die "unknown variant $v" ;;
    esac
    echo "== round $round: $v"
    local debug_env=()
    [ "$DIAG" = 1 ] && debug_env=(WAYLAND_DEBUG=server)
    # The wrapper sizes the host output, records its pid (which `exec` keeps,
    # so it is the compositor's) and the exec time, then becomes the
    # compositor. XDG_CONFIG_HOME points at an empty dir: scoot runs its
    # built-in defaults, and neither side picks up a user config.
    env "${debug_env[@]}" WLR_BACKENDS=headless WLR_RENDERER="$HOST_RENDERER" WLR_LIBINPUT_NO_DEVICES=1 \
        cage -d -- sh -c 'echo "$WAYLAND_DISPLAY" >"$0/host-display"
            wlr-randr --output HEADLESS-1 --custom-mode "$1" >/dev/null 2>&1
            echo $$ >"$0/pid"; date +%s%N >"$0/exec-ns"
            exec env -u WAYLAND_DEBUG XDG_CONFIG_HOME="$0/cfg" $2 >"$0/inner.log" 2>&1' \
        "$dir" "${WIDTH}x$HEIGHT" "$cmd" >"$HOST_LOG" 2>&1 &
    CAGE_PID=$!
    local i
    for i in $(seq 1 200); do [ -s "$dir/exec-ns" ] && break; sleep 0.01; done
    [ -s "$dir/exec-ns" ] || die "the host never started ($HOST_LOG)"
    PID=$(cat "$dir/pid")
    (
        sleep "$SESSION_TIMEOUT"
        if kill -0 "$PID" 2>/dev/null; then
            echo "  ($v, round $round: still running after ${SESSION_TIMEOUT}s; killed)" >>"$OUT/notes.log"
            kill "$PID"; sleep 2; kill -KILL "$PID" 2>/dev/null
        fi
    ) >/dev/null 2>&1 &
    local watchdog=$!
    local exec_ns ready_ns=""
    exec_ns=$(cat "$dir/exec-ns")
    if [ $kind = scoot ]; then
        export SCOOT_SOCKET="$dir/ipc.sock"
        act() { "$SCOOTCTL" action "$@"; }
        spawn() { timeout 5 "$SCOOTCTL" action spawn "$@"; }
        nwin() { timeout 5 "$SCOOTCTL" windows 2>/dev/null | grep -c '"app_id"'; }
        ready() { timeout 5 "$SCOOTCTL" version >/dev/null 2>&1; }
    else
        nsock() { ls "$XDG_RUNTIME_DIR"/niri.*."$PID".sock 2>/dev/null | head -1; }
        act() { "$NIRI" msg action "$@"; }
        spawn() { timeout 5 "$NIRI" msg action spawn -- "$@"; }
        nwin() { timeout 5 "$NIRI" msg windows 2>/dev/null | grep -c '^Window ID'; }
        ready() { NIRI_SOCKET=$(nsock); [ -n "$NIRI_SOCKET" ] && export NIRI_SOCKET && timeout 5 "$NIRI" msg version >/dev/null 2>&1; }
    fi
    for i in $(seq 1 1000); do
        if ready; then ready_ns=$(now_ns); break; fi
        sleep 0.005
    done
    [ -n "$ready_ns" ] || { kill "$PID" 2>/dev/null; die "$v never answered IPC ($dir/inner.log)"; }
    settle "$PID"
    local first="-"
    if [ "$DIAG" = 1 ]; then
        # The first get_xdg_surface on the host is the nested compositor's
        # (cage's other clients -- wlr-randr, Xwayland -- make none), and its
        # first attach+commit is the first frame. Host log stamps are
        # [HH:MM:SS.uuuuuu] wall-clock, as is exec-ns.
        SURF=$(grep -m1 -o 'get_xdg_surface(new id xdg_surface#[0-9]*, wl_surface#[0-9]*' "$HOST_LOG" | sed 's/.*wl_surface#//')
        if [ -n "$SURF" ]; then
            first=$(awk -v s="$SURF" -v e="$exec_ns" '
                index($0, "wl_surface#" s ".attach(wl_buffer#") { att = 1 }
                att && index($0, "wl_surface#" s ".commit()") {
                    split(substr($1, 2, length($1) - 2), t, ":")
                    ms = (t[1] * 3600 + t[2] * 60 + t[3]) * 1000
                    es = (e / 1e6) % 86400000
                    printf "%.1f", ms - es; exit }' "$HOST_LOG")
        fi
    fi
    printf '%s\t%s\t%s\t%s\n' "$round" "$v" \
        "$(awk -v a="$exec_ns" -v b="$ready_ns" 'BEGIN{printf "%.1f", (b-a)/1e6}')" "$first" >>"$OUT/startup.tsv"
    memory "$round" "$v" empty "$PID"

    spawn sh "$OUT/display.sh" "$dir/inner-display" >/dev/null 2>&1
    for i in 1 2 3; do spawn foot -c "$OUT/foot.ini" >/dev/null 2>&1; done
    for i in $(seq 1 100); do [ "$(nwin)" -ge 3 ] && break; sleep 0.1; done
    [ "$(nwin)" -ge 3 ] || echo "  (only $(nwin) windows mapped)" | tee -a "$OUT/notes.log"
    local inner_display host_display
    inner_display=$(cat "$dir/inner-display"); host_display=$(cat "$dir/host-display")
    settle "$PID"
    memory "$round" "$v" 3foot "$PID"

    scene_begin; sleep "$IDLE_SECS"; scene_end "$round" "$v" idle 0

    # Two points a half-output apart: with half-width columns they sit in
    # two different windows, so every event also moves pointer focus.
    settle "$PID"
    local out ev
    scene_begin
    out=$(WAYLAND_DISPLAY=$host_display "$VPTR" $((WIDTH / 4)) $((HEIGHT / 2)) $((WIDTH * 3 / 4)) $((HEIGHT / 2)) \
        "$WIDTH" "$HEIGHT" "$MOTION_HZ" "$MOTION_SECS" 500)
    ev=$(sed -n 's/^events=\([0-9]*\).*/\1/p' <<<"$out")
    scene_end "$round" "$v" pointer "${ev:-0}"

    # Six steps from the rightmost of three columns and back to it, each a
    # visible change: focus left, left, right, right, move left, right.
    settle "$PID"
    relayout_step() {
        local k=$(( $1 % 6 )) dir
        case $k in 0|1|4) dir=left ;; *) dir=right ;; esac
        if [ $k -lt 4 ]; then
            if [ $kind = scoot ]; then act focus-column $dir; else act focus-column-$dir; fi
        else
            if [ $kind = scoot ]; then act move-column $dir; else act move-column-$dir; fi
        fi
    }
    scene_begin
    ev=$(paced "$RELAYOUT_HZ" "$RELAYOUT_SECS" relayout_step)
    scene_end "$round" "$v" relayout "$ev"

    settle "$PID"
    local f t0 t1 k
    scene_begin
    for k in $(seq 1 "$SHOTS"); do
        f="$dir/shot-ipc-$k.png"
        t0=$(now_ns)
        if [ $kind = scoot ]; then
            "$SCOOTCTL" screenshot --no-cursor --out "$f" >/dev/null 2>&1
        else
            "$NIRI" msg action screenshot-screen --show-pointer false --path "$f" >/dev/null 2>&1
            # niri writes the file after it answers; wait for a whole PNG.
            for i in $(seq 1 2000); do png_done "$f" && break; sleep 0.001; done
        fi
        t1=$(now_ns)
        png_done "$f" || echo "  (shot $f incomplete)" | tee -a "$OUT/notes.log"
        printf '%s\t%s\tipc\t%s\t%s\t%s\n' "$round" "$v" "$k" \
            "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.2f", (b-a)/1e6}')" "$(stat -c %s "$f" 2>/dev/null || echo 0)" >>"$OUT/shots.tsv"
        sleep "$SHOT_GAP"
    done
    scene_end "$round" "$v" shot-ipc "$SHOTS"

    settle "$PID"
    scene_begin
    for k in $(seq 1 "$SHOTS"); do
        f="$dir/shot-grim-$k.png"
        t0=$(now_ns)
        WAYLAND_DISPLAY=$inner_display grim "$f" >/dev/null 2>&1
        t1=$(now_ns)
        printf '%s\t%s\tgrim\t%s\t%s\t%s\n' "$round" "$v" "$k" \
            "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.2f", (b-a)/1e6}')" "$(stat -c %s "$f" 2>/dev/null || echo 0)" >>"$OUT/shots.tsv"
        sleep "$SHOT_GAP"
    done
    scene_end "$round" "$v" shot-grim "$SHOTS"
    memory "$round" "$v" shots "$PID"
    # Keep the first and last of each kind as the visual record; the rest
    # are identical frames and only cost disk.
    for k in $(seq 2 $((SHOTS - 1))); do rm -f "$dir/shot-ipc-$k.png" "$dir/shot-grim-$k.png"; done

    settle "$PID"
    spawn foot -c "$OUT/foot.ini" sh "$OUT/anim.sh" >/dev/null 2>&1
    for i in $(seq 1 100); do [ "$(nwin)" -ge 4 ] && break; sleep 0.1; done
    sleep 1
    scene_begin; sleep "$ANIM_SECS"; scene_end "$round" "$v" animate "$ANIM_SECS"
    memory "$round" "$v" end "$PID"

    # Compositor first: cage waits for its child, and niri does not exit when
    # its host goes away -- killing cage first leaves niri running.
    kill "$PID" 2>/dev/null
    for i in $(seq 1 100); do kill -0 "$CAGE_PID" 2>/dev/null || break; sleep 0.05; done
    kill "$CAGE_PID" 2>/dev/null
    wait "$CAGE_PID" 2>/dev/null
    kill "$watchdog" 2>/dev/null
    unset SCOOT_SOCKET NIRI_SOCKET
    [ "$DIAG" = 1 ] && gzip -f "$HOST_LOG"
    sleep 1
}

trap '[ -n "${PID:-}" ] && kill "$PID" 2>/dev/null; [ -n "${CAGE_PID:-}" ] && kill "$CAGE_PID" 2>/dev/null' EXIT

read -r -a vs <<<"$VARIANTS"
for round in $(seq 1 "$ROUNDS"); do
    # Rotate the order each round so no variant always runs first or last.
    for j in $(seq 0 $((${#vs[@]} - 1))); do
        run_session "$round" "${vs[$(( (j + round - 1) % ${#vs[@]} ))]}"
    done
done
echo "raw numbers in $OUT"
