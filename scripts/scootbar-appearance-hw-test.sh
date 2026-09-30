#!/usr/bin/env bash
# scootbar's appearance options on a real compositor, real pixels: for each of
# four bar looks (flush-opaque, rounded-opaque, rounded-translucent, floating)
# start scoot and scootbar, capture a screenshot, sample pixels (corners are
# the desktop where they are cut, the bar's color where it should be), measure
# idle wakeups, CPU and RSS, measure what a whole-bar redraw costs (flush vs
# floating vs translucent), and check whether a click in a rounded corner
# reaches what is behind the bar. It ends with a PASS/FAIL/INFO table, and
# writes every raw number to files in OUT so they can be pasted into
# docs/scootbar/backlog/lightest.md's resource ratchet. Method and what to send
# back: docs/scootbar/testing.md ("The appearance hardware test").
#
#   MODE=--headless OUT=/tmp/sb-appearance scripts/scootbar-appearance-hw-test.sh
#   MODE=--nested   OUT=/tmp/sb-appearance scripts/scootbar-appearance-hw-test.sh
#   MODE=--tty      OUT=/tmp/sb-appearance scripts/scootbar-appearance-hw-test.sh
#
# MODE selects the scoot backend (default --headless, which needs no hardware
# and is how the harness itself is rehearsed). The numbers that matter are the
# --tty ones: dumb buffers scanned out by a real display, the CPU the
# compositor really spends re-compositing a translucent bar. --nested: run this
# script itself inside a host compositor (as scripts/smoke-test.sh's header
# says), so scoot finds its WAYLAND_DISPLAY.
#
# --tty, on the machine (a laptop whose desktop is not scoot is the low-risk
# case; treat anything else as needing your say-so, CLAUDE.md):
#
#   Ctrl+Alt+F3, log in, cd to this checkout
#   MODE=--tty OUT=/tmp/sb-appearance-tty scripts/scootbar-appearance-hw-test.sh
#
# **Stay on that VT until it finishes** (about 4 minutes at the defaults). A
# --tty compositor on an inactive VT is paused by logind: it holds no DRM
# master and draws nothing, so its numbers would look spectacular for the wrong
# reason. The script says so and marks the run when it sees the pause in the
# compositor's log. Ctrl+Alt+F<n> (the VT of your session) is the way back. The
# script takes no input.
#
# Prerequisites: built binaries (`cargo build --release -p scoot -p scootctl -p
# scootbar`; release is what the ratchet's numbers must come from, a debug
# build runs but is marked INFO: not for the ratchet), python3 (stdlib only:
# the PNG decoding and the pixel checks), `foot` on PATH (the client for the
# click check; without it that check is skipped and said so), and a font for
# the clock (any of scootbar's well-known ones; without one the bars run with
# no modules). No ImageMagick, jq or other tool.
#
# Overrides (all optional): SCOOT, SCOOTCTL, SCOOTBAR (binaries; the default is
# the release build then the debug build of the tree this script is in, or of
# $CARGO_TARGET_DIR), OUT (output directory; a directory that already holds a
# run is refused, OVERWRITE=1 replaces it), GAP (scoot's [layout] gap for the
# session, default 12), RADIUS (the bar's corner radius, default 12), BAR_HEIGHT
# (default 32), BAR (the bar's color, default #c03020), OPACITY (the
# translucent look's, default 0.5), IDLE_SECS (default 20), REDRAWS (whole-bar
# redraws per look, default 300), SWEEP_SECS (the paced cursor sweep along the
# bar per look, default 8), EXPECT_INPUT_REGION (honored, the default and
# what scoot does, or ignored: what the click check must see),
# CLICK_HEIGHT/CLICK_RADIUS (the click check's bar, default 120/60),
# SIZE (headless and nested only, WxH, default 1600x1000).
#
# Exit status: 0 with no FAIL, 1 with any FAIL, 2 for a setup problem (no
# binary, no seat, the compositor never came up), which is not a result.
set -euo pipefail

MODE=${MODE:---headless}
OUT=${OUT:-}
GAP=${GAP:-12}
RADIUS=${RADIUS:-12}
BAR_HEIGHT=${BAR_HEIGHT:-32}
BAR=${BAR:-#c03020}
OPACITY=${OPACITY:-0.5}
IDLE_SECS=${IDLE_SECS:-20}
REDRAWS=${REDRAWS:-300}
EXPECT_INPUT_REGION=${EXPECT_INPUT_REGION:-honored}
CLICK_HEIGHT=${CLICK_HEIGHT:-120}
CLICK_RADIUS=${CLICK_RADIUS:-60}
SIZE=${SIZE:-1600x1000}
SWEEP_SECS=${SWEEP_SECS:-8}

die() {
    echo "error: $*" >&2
    exit 2
}

case "$MODE" in
    --headless | --nested | --tty) ;;
    *) die "MODE must be --headless, --nested or --tty, not '$MODE'" ;;
esac
[ -n "$OUT" ] || die "set OUT to the directory the numbers go in (a new one)"
for n in GAP RADIUS BAR_HEIGHT IDLE_SECS REDRAWS SWEEP_SECS CLICK_HEIGHT CLICK_RADIUS; do
    case "${!n}" in
        '' | *[!0-9]*) die "$n must be a whole number, not '${!n}'" ;;
    esac
done
case "$BAR" in
    '#'[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]) ;;
    *) die "BAR must be '#rrggbb', not '$BAR'" ;;
esac
case "$EXPECT_INPUT_REGION" in
    honored | ignored) ;;
    *) die "EXPECT_INPUT_REGION must be honored or ignored" ;;
esac
command -v python3 >/dev/null || die "python3 is not on PATH (stdlib only, but it is needed)"
[ -n "${XDG_RUNTIME_DIR:-}" ] || die "no XDG_RUNTIME_DIR -- log in on a VT, do not su"

# --- which binaries this run tests (the smoke test's rule: the invoking tree,
# never a machine-specific path, and printed below so the record shows which).
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TARGET=${CARGO_TARGET_DIR:-$HERE/../target}
pick() {
    local name=$1 dir
    for dir in release debug; do
        if [ -x "$TARGET/$dir/$name" ]; then
            echo "$TARGET/$dir/$name"
            return
        fi
    done
}
SCOOT=${SCOOT:-$(pick scoot)}
SCOOTCTL=${SCOOTCTL:-$(pick scootctl)}
SCOOTBAR=${SCOOTBAR:-$(pick scootbar)}
[ -n "$SCOOT" ] && [ -x "$SCOOT" ] || die "no scoot binary (cargo build --release -p scoot, or set SCOOT)"
[ -n "$SCOOTBAR" ] && [ -x "$SCOOTBAR" ] || die "no scootbar binary (cargo build --release -p scootbar, or set SCOOTBAR)"
if [ -z "$SCOOTCTL" ] || [ ! -x "$SCOOTCTL" ]; then
    SCOOTCTL=
fi

# `scootctl`, or `scoot msg` (the same client) where no scootctl was built.
ctl() {
    if [ -n "$SCOOTCTL" ]; then
        "$SCOOTCTL" "$@"
    else
        "$SCOOT" msg "$@"
    fi
}

if [ -e "$OUT/summary.tsv" ] && [ "${OVERWRITE:-0}" != 1 ]; then
    echo "$OUT already holds a run ($OUT/summary.tsv exists); refusing to overwrite it." >&2
    echo "  measure afresh: OUT=<a new directory> $0" >&2
    echo "  overwrite it:   OVERWRITE=1 OUT=$OUT $0   (destroys the numbers in it)" >&2
    exit 2
fi
mkdir -p "$OUT"
# A private, short directory for the sockets: a unix socket path is capped at
# 107 bytes and OUT can be anywhere.
RUN=$(mktemp -d "$XDG_RUNTIME_DIR/sb-appearance.XXXXXX")
SOCK="$RUN/scoot.sock"
LOG="$OUT/compositor.log"

RESULTS="$OUT/results.txt"
SUMMARY="$OUT/summary.tsv"
PIXELS="$OUT/pixels.tsv"
: > "$RESULTS"
printf 'config\tbar_rss_kB\tidle_secs\tidle_bar_jiffies\tidle_bar_wakeups\tidle_scoot_jiffies\tredraws\tredraw_bar_jiffies\tredraw_scoot_jiffies\tredraw_wall_ms\tsweep_moves\tsweep_ms\tsweep_scoot_jiffies\n' > "$SUMMARY"
printf 'config\tstatus\tcheck\tdetail\n' > "$PIXELS"

# One row of the final table: STATUS (PASS, FAIL, INFO), what, detail.
row() {
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$RESULTS"
    printf '  %-4s %s: %s\n' "$1" "$2" "$3"
}

# --- the pixel helper: PNG decoding and the checks, stdlib only. Written next
# to the results so the record holds the exact code that judged them.
cat > "$OUT/pxl.py" <<'PY'
"""Pixel checks on scoot's PNG screenshots (8-bit RGB or RGBA, not
interlaced), stdlib only.

    pxl.py size FILE                       -> "W H"
    pxl.py sample FILE X Y                 -> "R G B A"
    pxl.py check FILE SPEC...              -> one "STATUS|label|detail" per SPEC

SPEC is label@x,y=want, want being #rrggbb, mix(#a,#b,alpha), or
not(#rrggbb) for "anything else", each optionally followed by ~tolerance
(default 3 levels a channel). Only the rows a check needs are decoded.
"""
import struct
import sys
import zlib


def read_png(path, rows_wanted=None):
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    pos, idat, header = 8, [], None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    width, height, depth, ctype, _, _, interlace = header
    if depth != 8 or ctype not in (2, 6) or interlace:
        raise SystemExit(f"{path}: need 8-bit RGB/RGBA, not interlaced: {header}")
    bpp = 3 if ctype == 2 else 4
    stride = width * bpp
    rows = height if rows_wanted is None else min(height, rows_wanted)
    # max_length 0 would mean "all of it": a zero-row read decodes nothing.
    raw = zlib.decompressobj().decompress(b"".join(idat), rows * (stride + 1)) if rows else b""
    out, prev = [], bytearray(stride)
    for y in range(rows):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if kind == 1:
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 255
        elif kind == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif kind == 3:
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 255
        elif kind == 4:
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
        elif kind != 0:
            raise SystemExit(f"{path}: bad filter {kind}")
        out.append(line)
        prev = line
    return width, height, bpp, out


def pixel(rows, bpp, x, y):
    r = rows[y]
    return (r[x * bpp], r[x * bpp + 1], r[x * bpp + 2], r[x * bpp + 3] if bpp == 4 else 255)


def hexrgb(text):
    text = text.lstrip("#")
    return tuple(int(text[i:i + 2], 16) for i in (0, 2, 4))


def want_of(want):
    """(rgb, negate) for a want expression."""
    if want.startswith("not(") and want.endswith(")"):
        return hexrgb(want[4:-1]), True
    if want.startswith("mix(") and want.endswith(")"):
        a, b, alpha = want[4:-1].split(",")
        a, b, alpha = hexrgb(a), hexrgb(b), float(alpha)
        return tuple(round(alpha * x + (1 - alpha) * y) for x, y in zip(a, b)), False
    return hexrgb(want), False


def main(argv):
    cmd = argv[1]
    if cmd == "size":
        w, h, _, _ = read_png(argv[2], 0)
        print(w, h)
    elif cmd == "sample":
        x, y = int(argv[3]), int(argv[4])
        w, h, bpp, rows = read_png(argv[2], int(argv[4]) + 1)
        print(*pixel(rows, bpp, x, y))
    elif cmd == "check":
        specs = []
        for spec in argv[3:]:
            label, rest = spec.split("@", 1)
            xy, want = rest.split("=", 1)
            tol = 3
            if "~" in want:
                want, t = want.rsplit("~", 1)
                tol = int(t)
            x, y = map(int, xy.split(","))
            specs.append((label, x, y, want, tol))
        need = max(y for _, _, y, _, _ in specs) + 1
        w, h, bpp, rows = read_png(argv[2], need)
        for label, x, y, want, tol in specs:
            if not (0 <= x < w and 0 <= y < len(rows)):
                print(f"FAIL|{label}|({x},{y}) is outside the {w}x{h} screenshot")
                continue
            got = pixel(rows, bpp, x, y)[:3]
            rgb, negate = want_of(want)
            near = all(abs(g - t) <= tol for g, t in zip(got, rgb))
            ok = (not near) if negate else near
            print(f"{'PASS' if ok else 'FAIL'}|{label}|({x},{y}) is rgb{got}, wanted {want} (+-{tol})")
    else:
        raise SystemExit(__doc__)


main(sys.argv)
PY
PXL=(python3 "$OUT/pxl.py")

# --- the record's cache key (CLAUDE.md: evidence is keyed to a tree state).
ENVLOG="$OUT/environment.txt"
{
    echo "date: $(date -Is)"
    echo "host: $(uname -srm)"
    echo "mode: $MODE"
    echo "harness tree: $(git -C "$HERE/.." rev-parse HEAD 2>/dev/null || echo '?') $(git -C "$HERE/.." status --porcelain 2>/dev/null | head -5 | tr '\n' ' ')"
    for b in "$SCOOT" "$SCOOTBAR"; do
        echo "binary: $b -> $(readlink -f "$b") ($(stat -c '%y' "$b" 2>/dev/null | cut -d. -f1)) $("$b" --version 2>&1 | head -1)"
    done
    echo "ctl: ${SCOOTCTL:-$SCOOT msg}"
    echo "settings: GAP=$GAP RADIUS=$RADIUS BAR_HEIGHT=$BAR_HEIGHT BAR=$BAR OPACITY=$OPACITY IDLE_SECS=$IDLE_SECS REDRAWS=$REDRAWS SIZE=$SIZE"
    echo "cpus: $(nproc)  clk_tck: $(getconf CLK_TCK)"
    echo "vt: $(fgconsole 2>/dev/null || echo '?')  session: ${XDG_SESSION_ID:-?}"
} > "$ENVLOG"
cat "$ENVLOG"

# A debug scootbar draws far slower than a release one: its numbers run, and
# say what they are.
case "$(readlink -f "$SCOOTBAR")" in
    */debug/*)
        row INFO "build" "scootbar is a DEBUG build: fine for the pixel checks, but the CPU and RSS numbers are not for the ratchet (build --release)"
        ;;
esac

# --- processes ---------------------------------------------------------------
COMP=
BAR_PID=
stop_bar() {
    if [ -n "$BAR_PID" ] && kill -0 "$BAR_PID" 2>/dev/null; then
        kill -TERM "$BAR_PID" 2>/dev/null || true
        for _ in $(seq 40); do
            kill -0 "$BAR_PID" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$BAR_PID" 2>/dev/null || true
        wait "$BAR_PID" 2>/dev/null || true
    fi
    BAR_PID=
}
cleanup() {
    stop_bar
    if [ -n "$COMP" ] && kill -0 "$COMP" 2>/dev/null; then
        # Its children first (a foot it spawned outlives it otherwise).
        pkill -TERM -P "$COMP" 2>/dev/null || true
        kill -TERM "$COMP" 2>/dev/null || true
        for _ in $(seq 40); do
            kill -0 "$COMP" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$COMP" 2>/dev/null || true
        wait "$COMP" 2>/dev/null || true
    fi
    rm -rf "$RUN"
}
trap 'cleanup; exit 130' INT TERM
trap cleanup EXIT

# utime+stime of a process in jiffies: fields 14 and 15 of /proc/PID/stat,
# read after the comm field, which can contain spaces.
jiffies() {
    [ -r "/proc/$1/stat" ] || {
        echo 0
        return
    }
    awk '{ s=$0; sub(/^[0-9]+ \(.*\) /, "", s); split(s, f, " "); print f[12] + f[13] }' "/proc/$1/stat"
}
status_field() { awk -v k="$2:" '$1 == k {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }
# Voluntary plus involuntary context switches: what a wakeup costs the
# scheduler, read the way the bench reads it.
ctxt() { echo $(($(status_field "$1" voluntary_ctxt_switches) + $(status_field "$1" nonvoluntary_ctxt_switches))); }
now_ms() { echo $(($(date +%s%N) / 1000000)); }

# scoot's own config for the session: the gap the floating look matches.
SCOOT_CONFIG="$RUN/scoot.toml"
printf '[layout]\ngap = %s\n' "$GAP" > "$SCOOT_CONFIG"

start_compositor() {
    local args=("$MODE")
    case "$MODE" in
        --headless | --nested)
            args+=(--width "${SIZE%x*}" --height "${SIZE#*x}")
            ;;
    esac
    # `env -u` so a stray SCOOT_SOCKET cannot aim this at a live session.
    env -u SCOOT_SOCKET -u WAYLAND_SOCKET "$SCOOT" "${args[@]}" --socket "$SOCK" \
        --config "$SCOOT_CONFIG" > "$LOG" 2>&1 &
    COMP=$!
    for _ in $(seq 100); do
        [ -S "$SOCK" ] && break
        kill -0 "$COMP" 2>/dev/null || break
        sleep 0.2
    done
    if [ ! -S "$SOCK" ]; then
        local last
        last=$(grep -aE '^scoot: |^error|panicked' "$LOG" | tail -1 || true)
        echo "the compositor never came up: ${last:-see $LOG}" >&2
        case "$last" in
            *libseat* | *"Permission denied"* | *busy* | *"seat "*)
                echo "  a busy SEAT, not a scootbar problem: another session holds it. Run this from the VT you are sitting on." >&2
                ;;
        esac
        exit 2
    fi
    export SCOOT_SOCKET="$SOCK"
    # The Wayland socket scoot listens on, off its own startup line (as
    # scripts/smoke-test.sh does).
    WAYLAND=$(sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" | grep 'scoot is up' | grep -o 'wayland-[0-9]*' | head -1 || true)
    [ -n "$WAYLAND" ] || die "no Wayland socket name in $LOG"
    export WAYLAND
    # For `scootbar msg` (the redraw storm), which finds its daemon by the
    # display.
    export WAYLAND_DISPLAY="$WAYLAND"
}

# --- bars --------------------------------------------------------------------
# A font the clock can use, so the bar carries a module like a real one; none
# means a solid bar with no modules.
FONT_OK=
for f in /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf /usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf \
    /usr/share/fonts/TTF/DejaVuSans.ttf /usr/share/fonts/truetype/DejaVuSans.ttf /usr/share/fonts/dejavu/DejaVuSans.ttf \
    /run/current-system/sw/share/X11/fonts/DejaVuSans.ttf /usr/share/fonts/truetype/noto/NotoSans-Regular.ttf \
    /usr/share/fonts/noto/NotoSans-Regular.ttf; do
    if [ -f "$f" ]; then
        FONT_OK=$f
        break
    fi
done
# The clock is centered, so it never touches a corner or an edge the pixel
# checks read (they sample the bar's left quarter, clear of it). Its minute
# tick is at most one redraw in the idle window.
if [ -n "$FONT_OK" ]; then
    CENTER=(--center clock)
else
    CENTER=(--center=)
    row INFO "font" "no well-known font found: the bars run with no modules (RSS then omits the font)"
fi

# Runs scootbar with config `$1` (a name), its [bar] lines in the remaining
# arguments, and waits until it has reserved its zone and drawn.
start_bar() {
    local name=$1
    shift
    local config="$OUT/bar-$name.toml"
    {
        echo '[bar]'
        printf '%s\n' "$@"
    } > "$config"
    (
        cd "$RUN"
        env WAYLAND_DISPLAY="$WAYLAND" \
            ${BAR_ENV:+"$BAR_ENV"} \
            "$SCOOTBAR" daemon --config "$config" --background "$BAR" "${CENTER[@]}" \
            > "$OUT/bar-$name.log" 2>&1 &
        echo $! > "$RUN/bar.pid"
    )
    BAR_PID=$(cat "$RUN/bar.pid")
    # Drawn: the zone is reserved (the usable area no longer starts at 0),
    # unless the look reserves none (BAR_ZONE=0: the click check's bar).
    local i
    for i in $(seq 150); do
        if [ "${BAR_ZONE:-1}" = 0 ]; then
            sleep 1
            break
        fi
        kill -0 "$BAR_PID" 2>/dev/null || {
            echo "scootbar exited early: $(tail -3 "$OUT/bar-$name.log")" >&2
            return 1
        }
        if ctl outputs 2>/dev/null | python3 -c '
import json, sys
o = json.load(sys.stdin)["outputs"][0]
sys.exit(0 if o["usable"]["y"] > 0 or o["usable"]["height"] < o["rect"]["height"] else 1)'; then
            break
        fi
        sleep 0.2
    done
    # And a frame committed: the compositor has the buffer after a round trip.
    ctl wait-idle --quiet-ms 300 --timeout-ms 8000 > /dev/null 2>&1 || true
}

# The screenshot of output 1, without the pointer, into $OUT/NAME.png.
shoot() {
    ctl screenshot --output 1 --no-cursor --out "$OUT/$1.png" > /dev/null
}

# `outputs`: prints "scale usable_y usable_height rect_w rect_h" of output 1.
geometry() {
    ctl outputs | python3 -c '
import json, sys
o = json.load(sys.stdin)["outputs"][0]
print(o["scale"], o["usable"]["y"], o["usable"]["height"], o["rect"]["width"], o["rect"]["height"])'
}

# Records the pixel checks in $PIXELS and the table. Arguments: config name,
# screenshot name, then specs for pxl.py.
pixel_checks() {
    local config=$1 shot=$2
    shift 2
    local status label detail
    while IFS='|' read -r status label detail; do
        printf '%s\t%s\t%s\t%s\n' "$config" "$status" "$label" "$detail" >> "$PIXELS"
        if [ "$status" = FAIL ]; then
            row FAIL "$config: $label" "$detail"
        fi
    done < <("${PXL[@]}" check "$OUT/$shot.png" "$@")
    local fails
    fails=$(awk -F'\t' -v c="$config" '$1 == c && $2 == "FAIL"' "$PIXELS" | wc -l)
    local total
    total=$(awk -F'\t' -v c="$config" '$1 == c' "$PIXELS" | wc -l)
    if [ "$fails" -eq 0 ]; then
        row PASS "$config: pixels" "$total of $total sampled pixels as expected ($OUT/$shot.png)"
    fi
}

# Device pixels of `$1` logical pixels at scale `$2`.
dev() { python3 -c "import sys; print(round(float(sys.argv[1]) * float(sys.argv[2])))" "$1" "$2"; }

# --- the four looks ------------------------------------------------------------
# Each measures, in this order: pixels, protocol trace (requests it makes),
# idle (wakeups, CPU, RSS), and a redraw storm (what a whole-bar redraw costs
# the bar and the compositor). Not traced while measured: WAYLAND_DEBUG would
# be most of the CPU.
measure_look() {
    local name=$1 desk_y
    shift
    echo "=== $name: [bar] $*"
    start_bar "$name" "$@" || {
        row FAIL "$name: start" "scootbar did not come up: $(tail -2 "$OUT/bar-$name.log" | tr '\n' ' ')"
        stop_bar
        return
    }
    shoot "$name"
    read -r SCALE UY UH RW RH < <(geometry)
    echo "  output ${RW}x${RH} logical, scale $SCALE, usable y=$UY height=$UH"

    # What the desktop looks like where the bar is not: sampled well below it.
    local size
    size=$("${PXL[@]}" size "$OUT/$name.png")
    local pw=${size% *}
    local h_dev
    h_dev=$(dev "$BAR_HEIGHT" "$SCALE")
    desk_y=$(dev "$((BAR_HEIGHT + 3 * GAP + 3 * RADIUS + 40))" "$SCALE")
    local desk
    desk=$("${PXL[@]}" sample "$OUT/$name.png" "$((pw / 2))" "$desk_y" | awk '{printf "#%02x%02x%02x", $1, $2, $3}')
    echo "  desktop color $desk (sampled at $((pw / 2)),$desk_y)"
    local r_dev x_far mid_y
    r_dev=$(dev "$RADIUS" "$SCALE")
    x_far=$((pw - 1))
    case "$name" in
        flush-opaque)
            # A flush bar: the corner pixel is the bar's, all four of them.
            mid_y=$((h_dev / 2))
            pixel_checks "$name" "$name" \
                "top-left corner@0,0=$BAR" "top-right corner@$x_far,0=$BAR" \
                "bottom-left corner@0,$((h_dev - 1))=$BAR" "bottom-right corner@$x_far,$((h_dev - 1))=$BAR" \
                "body@$((pw / 4)),$mid_y=$BAR" "below the bar@$((pw / 2)),$((h_dev + 2))=$desk"
            [ "$UY" = "$BAR_HEIGHT" ] || row FAIL "$name: zone" "usable area starts at y $UY, wanted the bar's height $BAR_HEIGHT"
            ;;
        rounded-opaque)
            mid_y=$((h_dev / 2))
            pixel_checks "$name" "$name" \
                "top-left corner cut@0,0=$desk" "top-right corner cut@$x_far,0=$desk" \
                "bottom-left corner cut@0,$((h_dev - 1))=$desk" "bottom-right corner cut@$x_far,$((h_dev - 1))=$desk" \
                "top edge past the corner@$((r_dev + 2)),0=$BAR" "left edge, middle@0,$mid_y=$BAR" \
                "body@$((pw / 4)),$mid_y=$BAR"
            ;;
        rounded-translucent)
            mid_y=$((h_dev / 2))
            pixel_checks "$name" "$name" \
                "top-left corner cut@0,0=$desk" "top-right corner cut@$x_far,0=$desk" \
                "blended body@$((pw / 4)),$mid_y=mix($BAR,$desk,$OPACITY)~6" \
                "blended left edge@0,$mid_y=mix($BAR,$desk,$OPACITY)~6"
            ;;
        floating)
            local m_dev
            m_dev=$(dev "$((GAP + RADIUS))" "$SCALE")
            mid_y=$((m_dev + h_dev / 2))
            pixel_checks "$name" "$name" \
                "top-left corner cut@$m_dev,$m_dev=$desk" "top-right corner cut@$((pw - 1 - m_dev)),$m_dev=$desk" \
                "screen corner is the desktop@0,0=$desk" "above the bar@$((pw / 2)),$((m_dev - 2))=$desk" \
                "left of the bar@$((m_dev - 2)),$mid_y=$desk" \
                "top edge past the corner@$((m_dev + r_dev + 2)),$m_dev=$BAR" "body@$((pw / 4)),$mid_y=$BAR"
            # The zone is the bar plus the margin on its edge.
            local want_y=$((BAR_HEIGHT + GAP + RADIUS))
            if [ "$UY" = "$want_y" ]; then
                row PASS "$name: zone" "windows start at y $UY = height $BAR_HEIGHT + margin $((GAP + RADIUS))"
            else
                row FAIL "$name: zone" "usable area starts at y $UY, wanted $want_y (height $BAR_HEIGHT + margin $((GAP + RADIUS)))"
            fi
            ;;
    esac

    # The protocol requests: the input region only when rounded, one
    # full-bar damage. Traced in a run of its own, then the measured bar is
    # started again untraced.
    stop_bar
    BAR_ENV=WAYLAND_DEBUG=1 start_bar "$name-trace" "$@" || true
    stop_bar
    local trace="$OUT/bar-$name-trace.log"
    local inputs damages
    inputs=$(grep -c 'set_input_region(' "$trace" || true)
    damages=$(grep -c 'damage_buffer(' "$trace" || true)
    echo "  trace: $inputs set_input_region, $damages damage_buffer"
    if [ "$name" = flush-opaque ]; then
        if [ "$inputs" -eq 0 ]; then
            row PASS "$name: input region" "left at the default (0 set_input_region requests)"
        else
            row FAIL "$name: input region" "$inputs set_input_region requests on a square bar"
        fi
    elif [ "$inputs" -ge 1 ]; then
        row PASS "$name: input region" "narrowed to the rounded shape ($inputs set_input_region request(s), $trace)"
    else
        row FAIL "$name: input region" "no set_input_region request from a rounded bar ($trace)"
    fi
    grep 'damage_buffer(' "$trace" | head -1 | sed 's/^.*damage_buffer/first damage: damage_buffer/' > "$OUT/$name.damage.txt" || true
    row INFO "$name: first damage" "$(cat "$OUT/$name.damage.txt" 2>/dev/null) ($damages damage_buffer request(s) in the run)"

    # Idle: the bar alone, nothing changing. Wait for the settled state first.
    start_bar "$name" "$@" || return
    sleep 3
    local bj0 bj1 sj0 sj1 c0 c1 rss
    bj0=$(jiffies "$BAR_PID")
    sj0=$(jiffies "$COMP")
    c0=$(ctxt "$BAR_PID")
    sleep "$IDLE_SECS"
    bj1=$(jiffies "$BAR_PID")
    sj1=$(jiffies "$COMP")
    c1=$(ctxt "$BAR_PID")
    rss=$(status_field "$BAR_PID" VmRSS)
    local idle_j=$((bj1 - bj0)) idle_c=$((c1 - c0)) idle_s=$((sj1 - sj0))
    row INFO "$name: idle" "scootbar ${idle_j} jiffies, ${idle_c} context switches, RSS ${rss} kB over ${IDLE_SECS}s; scoot ${idle_s} jiffies"
    # At most the clock's minute tick and the compositor's frame callbacks.
    if [ "$idle_c" -le 8 ] && [ "$idle_j" -le 2 ]; then
        row PASS "$name: idle wakeups" "$idle_c context switches and $idle_j jiffies in ${IDLE_SECS}s (limit 8 and 2)"
    else
        row FAIL "$name: idle wakeups" "$idle_c context switches and $idle_j jiffies in ${IDLE_SECS}s (limit 8 and 2): the bar is not idle"
    fi

    # A redraw storm: `reload` re-places every output, which draws the whole
    # bar again and makes the compositor take the new buffer. The same for
    # every look, so the differences are the look's: the fill, the blend the
    # compositor does (an opaque bar lets it skip blending under the bar),
    # the bigger ARGB buffer.
    local t0 t1 bj2 sj2 n
    sj0=$(jiffies "$COMP")
    bj0=$(jiffies "$BAR_PID")
    t0=$(now_ms)
    for n in $(seq "$REDRAWS"); do
        "$SCOOTBAR" msg reload > /dev/null 2>&1 || break
    done
    ctl wait-idle --quiet-ms 200 --timeout-ms 8000 > /dev/null 2>&1 || true
    t1=$(now_ms)
    bj2=$(jiffies "$BAR_PID")
    sj2=$(jiffies "$COMP")
    local redraws=$n
    row INFO "$name: redraws" "$redraws whole-bar redraws in $((t1 - t0)) ms: scootbar $((bj2 - bj0)) jiffies, scoot $((sj2 - sj0)) jiffies"

    # A cursor sweep along the bar, paced below the display's refresh rate so
    # each move is a frame of its own: the compositor re-composites the strip
    # under the cursor every frame, blending it when the bar is translucent
    # and skipping the blend under an opaque region. Scoot's CPU per move is
    # the compositor-side cost of the look (scootbar itself is idle).
    local moves=0 y x sweep0 sweep1 sj3 sj4
    y=$((BAR_HEIGHT / 2))
    if [ "$name" = floating ]; then
        y=$((y + GAP + RADIUS))
    fi
    sj3=$(jiffies "$COMP")
    sweep0=$(now_ms)
    while [ "$(($(now_ms) - sweep0))" -lt $((SWEEP_SECS * 1000)) ]; do
        moves=$((moves + 1))
        x=$((200 + (moves * 13) % 1000))
        ctl pointer move "$x" "$y" > /dev/null 2>&1
        sleep 0.0155
    done
    sweep1=$(now_ms)
    sj4=$(jiffies "$COMP")
    row INFO "$name: cursor sweep" "$moves moves along the bar in $((sweep1 - sweep0)) ms: scoot $((sj4 - sj3)) jiffies"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$rss" "$IDLE_SECS" "$idle_j" "$idle_c" "$idle_s" \
        "$redraws" "$((bj2 - bj0))" "$((sj2 - sj0))" "$((t1 - t0))" "$moves" "$((sweep1 - sweep0))" "$((sj4 - sj3))" >> "$SUMMARY"
    stop_bar
}

# --- the click in a rounded corner ---------------------------------------------
# A tall bar over two windows (it reserves nothing), corners cut by half its
# height. The left window's own corner lies under the bar's cut one: a click
# there reaches the window (focus moves to it) only if scoot honors the
# surface's input region; the bar's flat top swallows a click either way. The
# control is the same bar, square, which swallows both.
click_check() {
    echo "=== click in a rounded corner (bar ${CLICK_HEIGHT} high, radius $CLICK_RADIUS, expecting the region $EXPECT_INPUT_REGION)"
    if ! command -v foot > /dev/null; then
        row INFO "click" "skipped: no foot on PATH to put windows behind the bar"
        return
    fi
    local radius reached results=()
    for radius in "$CLICK_RADIUS" 0; do
        BAR_ZONE=0 start_bar "click-$radius" "height = $CLICK_HEIGHT" "radius = $radius" "exclusive = false" || {
            row FAIL "click: start" "scootbar did not come up"
            return
        }
        ctl action spawn foot > /dev/null 2>&1
        ctl action spawn foot > /dev/null 2>&1
        if ! reached=$(click_probe); then
            row INFO "click (radius $radius)" "skipped: $reached"
            results+=("skipped")
        else
            row INFO "click (radius $radius)" "$reached"
            results+=("$reached")
        fi
        close_windows
        stop_bar
    done
    local round=${results[0]:-skipped} square=${results[1]:-skipped}
    if [ "$round" = skipped ] || [ "$square" = skipped ]; then
        row INFO "click" "skipped: windows did not map (see the lines above)"
        return
    fi
    local round_reach=no square_reach=no
    case "$round" in reached*) round_reach=yes ;; esac
    case "$square" in reached*) square_reach=yes ;; esac
    if [ "$square_reach" = yes ]; then
        row FAIL "click: control" "a click in a SQUARE bar's corner reached the window: the harness is not measuring the bar"
    elif [ "$EXPECT_INPUT_REGION" = honored ]; then
        if [ "$round_reach" = yes ]; then
            row PASS "click: rounded corner" "reaches the window behind the bar (scoot honors the input region); a square bar swallows it"
        else
            row FAIL "click: rounded corner" "swallowed by the bar: scoot ignores the input region, or scootbar did not set it"
        fi
    elif [ "$round_reach" = no ]; then
        row PASS "click: rounded corner" "swallowed by the bar, as expected of a compositor that ignores input regions"
    else
        row FAIL "click: rounded corner" "reached the window, but EXPECT_INPUT_REGION=ignored"
    fi
}

# Closes every window (focused first, repeatedly) and waits until none is
# left, so the next look starts from an empty desktop.
close_windows() {
    local _
    for _ in $(seq 30); do
        ctl windows 2> /dev/null | grep -q '"id"' || return 0
        ctl action close > /dev/null 2>&1 || true
        sleep 0.3
    done
    echo "warning: windows still open after closing" >&2
}

# With two windows mapped and the bar up: prints "reached (...)" or "swallowed
# (...)" for a click in the left window's corner, after checking the flat
# part of the bar swallows one. Exit 1, saying why, if the windows never
# mapped. The bar draws over both windows; coordinates are logical, as the
# IPC's.
click_probe() {
    python3 - "$OUT" <<'PY' || exit 1
import json, os, subprocess, sys, time

def run(*args):
    cmd = os.environ["CTL_CMD"].split() + list(args)
    return subprocess.run(cmd, capture_output=True, text=True, timeout=20).stdout


def windows():
    try:
        return json.loads(run("windows")).get("windows", [])
    except (ValueError, subprocess.SubprocessError):
        return []


deadline = time.time() + 20
while time.time() < deadline and len(windows()) < 2:
    time.sleep(0.2)
ws = windows()
if len(ws) < 2:
    print(f"{len(ws)} window(s) mapped in 20 s")
    sys.exit(1)
run("wait-idle", "--quiet-ms", "400", "--timeout-ms", "8000")
ws = windows()
left = min(ws, key=lambda w: w["rect"]["x"])
if left["rect"]["x"] < 0 or left["rect"]["y"] < 0:
    print(f"the left window is off screen at {left['rect']['x']},{left['rect']['y']}")
    sys.exit(1)
focused = next((w for w in ws if w.get("focused")), None)
if focused is None or focused["id"] == left["id"]:
    print("the left window is already focused: nothing to move")
    sys.exit(1)
x, y = left["rect"]["x"], left["rect"]["y"]


def focus():
    f = next((w for w in windows() if w.get("focused")), None)
    return f["id"] if f else None


# The bar's flat top swallows a click: focus stays where it was.
run("pointer", "click", str(x + 200), str(y + 2), "left")
time.sleep(0.4)
flat = focus()
# The window's own corner, under the bar's cut one.
run("pointer", "click", str(x + 1), str(y + 1), "left")
deadline = time.time() + 2
while time.time() < deadline and focus() != left["id"]:
    time.sleep(0.1)
corner = focus()
detail = f"flat part -> focus {flat} (was {focused['id']}), corner ({x + 1},{y + 1}) -> focus {corner} (left window {left['id']})"
print(("reached (" if corner == left["id"] else "swallowed (") + detail + ")")
PY
}
if [ -n "$SCOOTCTL" ]; then
    export CTL_CMD="$SCOOTCTL"
else
    export CTL_CMD="$SCOOT msg"
fi

# --- run ---------------------------------------------------------------------------
echo "--- starting scoot ($MODE) ---"
start_compositor
echo "scoot up on $WAYLAND, control socket $SOCK"
ctl version > "$OUT/scoot-version.json" 2>&1 || true
ctl outputs > "$OUT/outputs.json" 2>&1 || true

measure_look flush-opaque "height = $BAR_HEIGHT"
measure_look rounded-opaque "height = $BAR_HEIGHT" "radius = $RADIUS"
measure_look rounded-translucent "height = $BAR_HEIGHT" "radius = $RADIUS" "opacity = $OPACITY"
measure_look floating "height = $BAR_HEIGHT" "margin = $((GAP + RADIUS))" "radius = $RADIUS"
click_check

# A --tty run that lost its VT measured a paused compositor.
if [ "$MODE" = --tty ] && grep -aq 'session paused' "$LOG"; then
    row FAIL "vt" "the VT was switched away during the run: the compositor was paused, none of these numbers count (rerun and stay on the VT)"
fi

# --- flush against floating, for lightest.md's ratchet ------------------------------
python3 - "$SUMMARY" "$REDRAWS" >> "$RESULTS" <<'PY' || true
import csv, sys

rows = {r["config"]: r for r in csv.DictReader(open(sys.argv[1]), delimiter="\t")}


def per(r, key):
    n = int(r["redraws"]) or 1
    return (int(r[key]) * 10.0) / n  # jiffies are 10 ms: ms per redraw


base = rows.get("flush-opaque")
if base:
    for name in ("rounded-opaque", "rounded-translucent", "floating"):
        r = rows.get(name)
        if not r:
            continue
        d_bar = per(r, "redraw_bar_jiffies") - per(base, "redraw_bar_jiffies")
        d_scoot = per(r, "redraw_scoot_jiffies") - per(base, "redraw_scoot_jiffies")
        wall = (int(r["redraw_wall_ms"]) - int(base["redraw_wall_ms"])) / (int(r["redraws"]) or 1)
        print(f"INFO\t{name} against flush-opaque, per whole-bar redraw\tscootbar {d_bar:+.2f} ms CPU, scoot {d_scoot:+.2f} ms CPU, wall {wall:+.2f} ms")
        moves = int(r["sweep_moves"]) or 1
        bmoves = int(base["sweep_moves"]) or 1
        s_now = int(r["sweep_scoot_jiffies"]) * 10.0 / moves
        s_base = int(base["sweep_scoot_jiffies"]) * 10.0 / bmoves
        print(f"INFO\t{name} against flush-opaque, per cursor move\tscoot {s_now:.2f} ms CPU against {s_base:.2f} ms ({s_now - s_base:+.2f}); 10 ms jiffies over {moves} and {bmoves} moves: trust a difference only past a jiffy per hundred moves")
PY

# --- the table ------------------------------------------------------------------------
echo
echo "================================ RESULT ($MODE) ================================"
awk -F'\t' '{ printf "%-4s  %-46s %s\n", $1, $2, $3 }' "$RESULTS"
echo "------------------------------------------------------------------------------"
pass=$(awk -F'\t' '$1 == "PASS"' "$RESULTS" | wc -l)
fail=$(awk -F'\t' '$1 == "FAIL"' "$RESULTS" | wc -l)
info=$(awk -F'\t' '$1 == "INFO"' "$RESULTS" | wc -l)
echo "PASS $pass   FAIL $fail   INFO $info"
echo "raw numbers: $SUMMARY (idle and redraw per look), $PIXELS (every sampled pixel),"
echo "  $ENVLOG (the cache key), screenshots $OUT/*.png, protocol traces $OUT/bar-*-trace.log"
echo "send back: the OUT directory (or summary.tsv, pixels.tsv, environment.txt and results.txt)."
[ "$fail" -eq 0 ]
