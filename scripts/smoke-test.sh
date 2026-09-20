#!/usr/bin/env bash
# Drives scoot end to end over IPC only: start it, open a terminal inside it,
# type into that terminal, and capture the screen. Backend-agnostic by design
# -- IPC input/introspection works the same regardless of how the compositor
# presents itself, so this is the regression net for every backend, not just
# --headless.
#
# MODE selects the backend (default --headless); RENDERER selects which
# renderer draws the frames it checks (default pixman, or `gles` under
# --headless/--nested), e.g. RENDERER=gles scripts/smoke-test.sh. For
# --nested, run this script itself inside a host compositor, e.g.:
#   WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
#     cage -- env MODE=--nested scripts/smoke-test.sh
# so scoot's own Connection::connect_to_env() (nested.rs) finds cage's
# WAYLAND_DISPLAY when it starts, not this shell's.
#
# Every temp path this script uses derives from one overridable prefix, so
# two concurrent runs (e.g. two agents verifying different branches on the
# same VM) can't collide: run one as SMOKE_PREFIX=/tmp/smoke-a
# scripts/smoke-test.sh and the other as SMOKE_PREFIX=/tmp/smoke-b
# scripts/smoke-test.sh and every socket, log, screenshot, config and
# scratch file below becomes /tmp/smoke-a* vs /tmp/smoke-b*. Two runs
# sharing one prefix still collide -- prefixes must differ. An explicit
# SOCKET/SHOT/LOG/CONFIG/TYPED still wins over the prefix-derived default
# for that one path, so existing callers that set SOCKET/LOG/MODE/SCOOT
# keep working; with SMOKE_PREFIX unset every default is exactly the
# hardcoded path below. A trailing slash on the prefix is stripped, a
# nonexistent parent dir is created (mkdir -p), and every expansion is
# quoted so a prefix containing spaces works.
#
# SCOOT and SCOOTCTL pick which binaries this run tests: explicit values
# always win; otherwise SCOOT defaults to the invoking tree
# ($CARGO_TARGET_DIR/debug/scoot when set, else
# <this repo>/target/debug/scoot resolved from this script's own location,
# so the script runs from any cwd) and SCOOTCTL to the scootctl next to
# it -- the same build, never a split-brain pair. Either default missing
# (or a bare name not on PATH) fails loudly before anything launches, and
# the run's first lines print each binary's path, source and mtime -- read
# them and confirm the binary is your build, especially where
# CARGO_TARGET_DIR is shared between checkouts.
set -euo pipefail

MODE=${MODE:---headless}
# Which binaries this run tests. An explicit SCOOT/SCOOTCTL always wins, so
# existing callers and CI (which set both) see no change.
#
# The SCOOT default is the invoking tree, never a machine-specific absolute
# path: $CARGO_TARGET_DIR/debug/scoot when that is set (the builder's real
# output dir -- export a private one and the default follows your build),
# else <this-script's-repo>/target/debug/scoot, resolved from this script's
# own location rather than the cwd, so the script runs from anywhere. A
# globally shared CARGO_TARGET_DIR (the dev VM points every checkout at
# /var/cargo-target) still resolves to the shared binary -- the header below
# then shows exactly which file that is, so read it and confirm it is your
# build before trusting a green run.
#
# SCOOTCTL defaults to the scootctl next to SCOOT -- the same build. A
# compositor from one tree driven by a client from another is the same
# wrong-verdict class this default exists to prevent, so the pairing is
# kept, not split: there is no SCOOTCTL default independent of SCOOT. A bare
# SCOOT (found on PATH rather than naming a file) is pinned to its full
# path first, so the pairing, the checks and the header all see one file --
# dirname of a bare name would be ".", i.e. the cwd, which is the wrong tree
# by construction.
#
# Either binary missing, not a regular file, or not executable fails loudly
# here, before anything launches, instead of testing someone else's binary
# -- or nothing at all -- and calling it green.
if [ -n "${SCOOT:-}" ]; then
    SCOOT_SRC="environment"
elif [ -n "${CARGO_TARGET_DIR:-}" ]; then
    SCOOT="$CARGO_TARGET_DIR/debug/scoot"
    SCOOT_SRC="default from CARGO_TARGET_DIR"
else
    SCOOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../target/debug/scoot"
    SCOOT_SRC="default from the invoking tree"
fi
case "$SCOOT" in
    */*) ;;
    *)
        SCOOT_ON_PATH=$(command -v "$SCOOT" || true)
        if [ -z "$SCOOT_ON_PATH" ]; then
            echo "error: SCOOT='$SCOOT' names no file and is not on PATH -- set SCOOT to the compositor binary to test"
            exit 1
        fi
        case "$SCOOT_ON_PATH" in
            */*) SCOOT="$SCOOT_ON_PATH" ;;
            *) SCOOT="$PWD/$SCOOT_ON_PATH" ;;
        esac
        SCOOT_SRC="$SCOOT_SRC (on PATH at $SCOOT)"
        ;;
esac
if [ ! -f "$SCOOT" ] || [ ! -x "$SCOOT" ]; then
    echo "error: no compositor binary at SCOOT=$SCOOT ($SCOOT_SRC) -- set SCOOT explicitly or build first (cargo build -p scoot)"
    exit 1
fi
if [ -n "${SCOOTCTL:-}" ]; then
    SCOOTCTL_SRC="environment"
else
    SCOOTCTL="$(dirname "$SCOOT")/scootctl"
    SCOOTCTL_SRC="default next to SCOOT (same build)"
fi
if [ ! -f "$SCOOTCTL" ] || [ ! -x "$SCOOTCTL" ]; then
    echo "error: no client binary at SCOOTCTL=$SCOOTCTL ($SCOOTCTL_SRC) -- the split must ship both binaries; set SCOOTCTL explicitly or build first (cargo build -p scootctl)"
    exit 1
fi
echo "--- binaries ---"
echo "SCOOT=$SCOOT ($SCOOT_SRC)"
ls -l "$SCOOT"
echo "SCOOTCTL=$SCOOTCTL ($SCOOTCTL_SRC)"
ls -l "$SCOOTCTL"
# RENDERER picks which renderer composites the frames this run checks
# (`pixman` -- the default when unset -- or `gles`, which needs --headless or
# --nested; see README). Unset means the flag is not passed at all, so the
# default run is byte-for-byte the command this script has always used. Only
# the main compositor below takes it: the config-fallback launches further
# down check config parsing, not pixels.
RENDERER=${RENDERER:-}
RENDERER_ARGS=()
if [ -n "$RENDERER" ]; then
    RENDERER_ARGS=(--renderer "$RENDERER")
fi
SMOKE_PREFIX=${SMOKE_PREFIX:-}
while [ -n "$SMOKE_PREFIX" ] && [ "$SMOKE_PREFIX" != "/" ] && [ "${SMOKE_PREFIX%/}" != "$SMOKE_PREFIX" ]; do
    SMOKE_PREFIX=${SMOKE_PREFIX%/}
done
if [ -n "$SMOKE_PREFIX" ]; then
    mkdir -p "$(dirname "$SMOKE_PREFIX")"
    SOCKET=${SOCKET:-"$SMOKE_PREFIX.sock"}
    SHOT=${SHOT:-"$SMOKE_PREFIX.png"}
    LOG=${LOG:-"$SMOKE_PREFIX.log"}
    CONFIG=${CONFIG:-"$SMOKE_PREFIX-appearance.toml"}
    TYPED_DEFAULT="$SMOKE_PREFIX-typed.txt"
    KILLED="$SMOKE_PREFIX-killed.png"
    CONFIG_SOCK="$SMOKE_PREFIX-config.sock"
    CONFIG_LOG="$SMOKE_PREFIX-config.log"
    CAPITAL_SOCK="$SMOKE_PREFIX-capital.sock"
    CAPITAL_LOG="$SMOKE_PREFIX-capital.log"
    BROKEN_SOCK="$SMOKE_PREFIX-broken.sock"
    BROKEN_LOG="$SMOKE_PREFIX-broken.log"
    CONFIG_TEMPLATE="$SMOKE_PREFIX-config-XXXXXX.toml"
    CAPITAL_TEMPLATE="$SMOKE_PREFIX-capital-XXXXXX.toml"
    BROKEN_TEMPLATE="$SMOKE_PREFIX-broken-XXXXXX.toml"
else
    SOCKET=${SOCKET:-/run/user/$(id -u)/scoot-smoke.sock}
    SHOT=${SHOT:-/tmp/scoot-smoke.png}
    LOG=${LOG:-/tmp/scoot-smoke.log}
    CONFIG=${CONFIG:-/tmp/scoot-smoke-appearance.toml}
    TYPED_DEFAULT=/tmp/scoot-smoke-typed.txt
    KILLED=/tmp/scoot-smoke-killed.png
    CONFIG_SOCK="/run/user/$(id -u)/scoot-smoke-config.sock"
    CONFIG_LOG=/tmp/scoot-smoke-config.log
    CAPITAL_SOCK="/run/user/$(id -u)/scoot-smoke-capital.sock"
    CAPITAL_LOG=/tmp/scoot-smoke-capital.log
    BROKEN_SOCK="/run/user/$(id -u)/scoot-smoke-broken.sock"
    BROKEN_LOG=/tmp/scoot-smoke-broken.log
    CONFIG_TEMPLATE=/tmp/scoot-smoke-config-XXXXXX.toml
    CAPITAL_TEMPLATE=/tmp/scoot-smoke-capital-XXXXXX.toml
    BROKEN_TEMPLATE=/tmp/scoot-smoke-broken-XXXXXX.toml
fi
export SCOOT_SOCKET="$SOCKET"

rm -f "$SOCKET" "$SHOT" "$LOG" "$CONFIG"

# A deliberately distinctive, non-default [appearance] -- so the pixel checks
# below can't pass by accident against whatever the built-in defaults happen
# to be. gap stays at its own default (12): RING_WIDTH is exactly half of
# that, right at the clamp boundary (see decorations.rs's
# Appearance::clamped) without tripping it, so this also doubles as a check
# that a legal, non-clamped width isn't clamped anyway.
#
# A single variable, not a literal repeated in the config below and again in
# the pixel-sampling math further down -- two independent copies of "6" would
# only need one of them edited to silently break the sampling geometry.
RING_WIDTH=6
cat >"$CONFIG" <<EOF
[appearance]
focus_ring_width = $RING_WIDTH
focus_ring_active_color = "#ff00ff"
focus_ring_inactive_color = "#00ffff"
background_color = "#123456"
EOF

# Reads one pixel out of a PNG as "R G B" (0-255), via whichever of
# magick/convert is on PATH, falling back to a throwaway `nix shell` when
# neither is (e.g. before vm/configuration.nix's `pkgs.imagemagick` addition
# has been picked up by a VM rebuild -- see that file's comment on this).
magick_cmd() {
    if command -v magick >/dev/null 2>&1; then
        magick "$@"
    elif command -v convert >/dev/null 2>&1; then
        convert "$@"
    else
        nix shell nixpkgs#imagemagick -c magick "$@"
    fi
}

read_pixel() {
    local png="$1" x="$2" y="$3"
    local raw inner
    # -depth 8 forces 0-255 integer channel output regardless of this
    # ImageMagick build's own default quantum depth (some builds are Q16,
    # which without this would print e.g. percentages or a 16-bit scale
    # instead) -- our own PNGs (screenshot.rs) are already 8-bit, so this is
    # a no-op for them and only guards against a differently-built magick.
    raw=$(magick_cmd "$png" -depth 8 -format '%[pixel:p{'"$x"','"$y"'}]' info:)
    inner=${raw#*(}
    inner=${inner%)*}
    IFS=',' read -r -a channels <<<"$inner"
    echo "${channels[0]} ${channels[1]} ${channels[2]}"
}

# Asserts the pixel at ($x,$y) in $png is $want_hex ("#rrggbb"), within a
# small tolerance (compositing/format round-tripping, not a hue difference)
# -- a hard mismatch is a real bug, not compositor noise.
expect_pixel_color() {
    local png="$1" x="$2" y="$3" want_hex="$4" label="$5"
    local want_r want_g want_b got_r got_g got_b
    want_r=$((16#${want_hex:0:2}))
    want_g=$((16#${want_hex:2:2}))
    want_b=$((16#${want_hex:4:2}))
    read -r got_r got_g got_b < <(read_pixel "$png" "$x" "$y")
    local dr dg db
    dr=$(( want_r > got_r ? want_r - got_r : got_r - want_r ))
    dg=$(( want_g > got_g ? want_g - got_g : got_g - want_g ))
    db=$(( want_b > got_b ? want_b - got_b : got_b - want_b ))
    if [ "$dr" -gt 2 ] || [ "$dg" -gt 2 ] || [ "$db" -gt 2 ]; then
        echo "BUG: $label pixel at ($x,$y) is rgb($got_r,$got_g,$got_b), expected #$want_hex (rgb($want_r,$want_g,$want_b))"
        return 1
    fi
    echo "ok: $label pixel at ($x,$y) matches #$want_hex"
}

"$SCOOT" "$MODE" --width 1200 --height 800 "${RENDERER_ARGS[@]}" --socket "$SOCKET" \
    --config "$CONFIG" >"$LOG" 2>&1 &
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
"$SCOOT" msg version

echo "--- opening a terminal ---"
"$SCOOT" msg action spawn foot

echo "--- waiting for it to appear as a window ---"
mapped=0
for _ in $(seq 1 100); do
    if "$SCOOT" msg windows | grep -q '"app_id"'; then
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
"$SCOOT" msg windows

echo "--- scoot msg vs scootctl: same request, identical answer ---"
# The alias is the same client (it parses and runs through the scootctl
# crate), so a request answered from either binary must be byte-identical on
# stdout -- and a bad one must fail identically (same exit code, same message
# past the `scoot: ` / `scootctl: ` prefix). `windows` is compared here, with
# two terminals mapped, rather than above, so the reply is non-empty. (Both
# binaries were checked present and executable up front in the resolution
# block, so a failure here is a real divergence, not a missing file.)
# One request answered by both clients: same stdout bytes AND same exit code.
# `cmp <(...)` alone would pass vacuously if both sides failed identically, so
# the codes are captured alongside the bytes (the `|| code=$?` guard, not a
# bare capture: a failing client trips `set -e`, as above).
same_answer() {
    desc=$1; shift
    prefix=${SMOKE_PREFIX:-/tmp/scoot-smoke}
    alias_out="$prefix-equiv-alias.out"
    ctl_out="$prefix-equiv-ctl.out"
    alias_code=0
    "$SCOOT" msg "$@" >"$alias_out" || alias_code=$?
    ctl_code=0
    "$SCOOTCTL" "$@" >"$ctl_out" || ctl_code=$?
    cmp "$alias_out" "$ctl_out" \
        || { echo "BUG: $desc differs between scoot msg and scootctl"; exit 1; }
    if [ "$alias_code" != "$ctl_code" ]; then
        echo "BUG: $desc exits $alias_code via scoot msg but $ctl_code via scootctl"
        exit 1
    fi
}
same_answer "version" version
same_answer "windows" windows
same_answer "outputs" outputs
# Screenshots of the same settled screen must be the same PNG, byte for byte.
"$SCOOT" msg wait-idle --quiet-ms 500 --timeout-ms 10000
same_answer "screenshot bytes" screenshot
# `action` replies and malformed-arg errors, same treatment.
same_answer "an action reply" action focus-column left
for bad in "frobnicate" "pointer move x 1" "action focus-column sideways"; do
    # `|| code=$?` (not a bare capture): a failing client inside `$(...)`
    # trips `set -e` on the assignment itself, so the exit code is taken in
    # the guard. `$bad` splits on purpose -- each entry is a word list.
    alias_code=0
    # shellcheck disable=SC2086
    alias_err=$("$SCOOT" msg $bad 2>&1) || alias_code=$?
    ctl_code=0
    # shellcheck disable=SC2086
    ctl_err=$("$SCOOTCTL" $bad 2>&1) || ctl_code=$?
    if [ "$alias_code" = "0" ] || [ "$ctl_code" = "0" ]; then
        echo "BUG: '$bad' unexpectedly succeeded from one client ($alias_code vs $ctl_code)"
        exit 1
    fi
    if [ "$alias_code" != "$ctl_code" ] || [ "${alias_err#scoot: }" != "${ctl_err#scootctl: }" ]; then
        echo "BUG: '$bad' fails differently between scoot msg and scootctl"
        echo "  scoot msg: exit $alias_code: $alias_err"
        echo "  scootctl:  exit $ctl_code: $ctl_err"
        exit 1
    fi
done
echo "ok: scoot msg and scootctl answer identically"

echo "--- checking the spawned terminal got an activation token ---"
# State::spawn mints an xdg-activation token per child
# (XDG_ACTIVATION_TOKEN). Exactly one foot exists this early in the script,
# so the newest match is unambiguous.
foot_pid=$(pgrep -n -x foot)
if ! tr '\0' '\n' <"/proc/$foot_pid/environ" | grep -q '^XDG_ACTIVATION_TOKEN=.\+$'; then
    echo "BUG: the foot State::spawn started carries no XDG_ACTIVATION_TOKEN"
    exit 1
fi
echo "ok: the spawned terminal carries XDG_ACTIVATION_TOKEN"

echo "--- typing ---"
"$SCOOT" msg type 'hello from scoot'
"$SCOOT" msg wait-idle --quiet-ms 500 --timeout-ms 10000

echo "--- checking the typed text actually reached the client ---"
"$SCOOT" msg screenshot --out "$SHOT.check1" >/dev/null
"$SCOOT" msg type 'x'
"$SCOOT" msg wait-idle --quiet-ms 500 --timeout-ms 10000
"$SCOOT" msg screenshot --out "$SHOT.check2" >/dev/null
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
"$SCOOT" msg type "$(printf 'echo one\necho two')"
"$SCOOT" msg wait-idle --quiet-ms 500 --timeout-ms 10000

echo "--- typing shifted characters, round-tripped back out of the terminal ---"
# Every category the level-0 shift bug got wrong (2026-09-13): capitals and
# shifted punctuation each arrived as their unshifted twin -- `AbC` as `abc`,
# `!` as `1` -- with no error, because the shift decision asked a Smithay
# helper that only ever reports level 0. A screenshot can't catch that (the
# wrong text is still text), so this reads the characters back out of the
# shell running in the terminal and compares them byte for byte. The `>`
# redirect is itself a shifted character, so the check could not even be
# written without the fix.
NEWLINE=$'\n'
SHIFTED='AbC_xyz !?~:|@#$%^&*()+{}<>"'
TYPED=${TYPED:-"$TYPED_DEFAULT"}
rm -f "$TYPED"
# The steps above left `echo two` sitting at the prompt unexecuted; run it so
# this one starts on an empty command line.
"$SCOOT" msg type "$NEWLINE"
"$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
"$SCOOT" msg type "printf '%s' '$SHIFTED' > \"$TYPED\"$NEWLINE"
"$SCOOT" msg wait-idle --quiet-ms 500 --timeout-ms 10000
for _ in $(seq 1 50); do
    [ -s "$TYPED" ] && break
    sleep 0.2
done
if [ ! -s "$TYPED" ]; then
    echo "BUG: the command typed into the terminal never wrote $TYPED"
    echo "(the shell in the terminal saw something other than what was typed)"
    exit 1
fi
if [ "$(cat "$TYPED")" != "$SHIFTED" ]; then
    echo "BUG: typed text came back wrong"
    echo "  typed:    $SHIFTED"
    echo "  received: $(cat "$TYPED")"
    exit 1
fi
echo "ok: every shifted character arrived exactly as typed"
rm -f "$TYPED"

focused_window() {
    "$SCOOT" msg windows | jq -r '.windows[] | select(.focused) | .id'
}

echo "--- opening a second terminal, to exercise keybindings ---"
"$SCOOT" msg action spawn foot
mapped=0
for _ in $(seq 1 100); do
    if [ "$("$SCOOT" msg windows | jq '.windows | length')" -eq 2 ]; then
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
first_id=$("$SCOOT" msg windows | jq -r --argjson id "$second_id" '.windows[] | select(.id != $id) | .id')

echo "--- a bare 'h' has no binding: it types into the focused terminal, focus stays put ---"
"$SCOOT" msg key h
"$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
if [ "$(focused_window)" != "$second_id" ]; then
    echo "BUG: a bare 'h' (no modifier) moved focus -- keybindings must not fire without Super"
    exit 1
fi
echo "ok: focus unchanged by a bare 'h'"

echo "--- super+h moves focus to the column on the left (Action::FocusColumn) ---"
"$SCOOT" msg key super+h
"$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
if [ "$(focused_window)" != "$first_id" ]; then
    echo "BUG: super+h did not move focus to the other column; keybindings may be broken"
    "$SCOOT" msg windows
    exit 1
fi
echo "ok: super+h moved focus from window $second_id to window $first_id"

# At this point (after the super+h test above) $first_id is focused and
# $second_id is not. Sampling the *top* of each window's ring rather than a
# side facing the other window sidesteps needing to reason about which side
# has a neighbor nearby -- the layout always leaves a full gap above every
# window on a single row, focused or not, regardless of how many columns
# there are or which one is scrolled into view.
windows_json=$("$SCOOT" msg windows)
read -r focused_x focused_y focused_w focused_h < <(
    echo "$windows_json" | jq -r --argjson id "$first_id" \
        '.windows[] | select(.id == $id) | "\(.rect.x) \(.rect.y) \(.rect.width) \(.rect.height)"'
)
read -r unfocused_x unfocused_y unfocused_w < <(
    echo "$windows_json" | jq -r --argjson id "$second_id" \
        '.windows[] | select(.id == $id) | "\(.rect.x) \(.rect.y) \(.rect.width)"'
)

echo "--- parking the pointer clear of everything sampled below ---"
# --tty is the one backend that draws a cursor (see cursor.rs), and the
# pointer starts at the output's origin -- where the built-in arrow's opaque
# black outline runs diagonally straight through the background sample at
# (3,3) below, which is why that check read rgb(0,0,0) under --tty alone
# while --headless/--nested passed. Parking the pointer is what makes that
# sample measure the background; moving the sample instead would bake today's
# cursor placement and size into a coordinate that has no reason to know
# them. The focused window's middle is hundreds of pixels from all three
# sampled pixels, and motion never moves keyboard focus here (input.rs has no
# focus-follows-mouse), so the ring colors below still describe the focus
# super+h left behind.
"$SCOOT" msg pointer move "$((focused_x + focused_w / 2))" "$((focused_y + focused_h / 2))"
"$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000

echo "--- screenshot ---"
"$SCOOT" msg screenshot --out "$SHOT"

echo "=== decorations: focus ring + background render the configured colors ==="
ring_ok=1
expect_pixel_color "$SHOT" \
    "$((focused_x + focused_w / 2))" "$((focused_y - RING_WIDTH / 2))" \
    "ff00ff" "the focused window's ring" || ring_ok=0
expect_pixel_color "$SHOT" \
    "$((unfocused_x + unfocused_w / 2))" "$((unfocused_y - RING_WIDTH / 2))" \
    "00ffff" "the unfocused window's ring" || ring_ok=0
# Halfway between the output's own corner and the *outer* edge of the
# focused window's ring (not the window's own rect -- landing exactly on
# that boundary would sample the ring itself, not the background past it).
# $focused_x/$focused_y equal the layout's gap here (the leftmost/topmost
# column sits exactly `gap` from the output's edge), so this stays clear of
# both the top and left ring segments regardless of what gap or
# focus_ring_width actually are, rather than assuming today's defaults.
expect_pixel_color "$SHOT" \
    "$(((focused_x - RING_WIDTH) / 2))" "$(((focused_y - RING_WIDTH) / 2))" \
    "123456" "the background" || ring_ok=0
if [ "$ring_ok" -ne 1 ]; then
    echo "BUG: one or more decoration pixel checks failed -- see above"
    exit 1
fi

echo "--- clients killed mid-screenshot do not wedge the compositor ---"
# Each iteration asks for a screenshot and is killed at once: depending on
# the race the client dies before connecting, mid-encode (the orphaned
# capture is dropped when its reply finds a dead peer), or after its reply
# already arrived. All three are safe by construction; what must hold
# afterwards is that the compositor still answers. The harness suite pins
# the mid-encode case deterministically (a_client_that_disconnects_mid_encode);
# this hammers every interleaving end to end instead.
for _ in $(seq 1 20); do
    "$SCOOT" msg screenshot --out "$KILLED" >/dev/null 2>&1 &
    killer=$!
    killed="${killed-} $killer"
    kill -9 "$killer" 2>/dev/null || true
done
# Only these: a bare `wait` would also wait for the compositor itself, which
# never exits on its own, and hang the script.
# shellcheck disable=SC2086
wait $killed 2>/dev/null || true
rm -f "$KILLED"
"$SCOOT" msg version >/dev/null || {
    echo "BUG: the compositor stopped answering after clients were killed mid-screenshot"
    tail -30 "$LOG"
    exit 1
}
echo "ok: the compositor still answers after 20 clients killed mid-screenshot"

echo "--- checking clients see zwp_linux_dmabuf_v1 ---"
# The dmabuf advertisement (see dmabuf.rs) is bind-time state with no IPC
# surface, so the only end-to-end proof the global is really advertised is
# asking the registry itself. The Wayland socket name is auto-assigned and
# logged at startup ("scoot is up" carries it as `wayland="..."`).
if command -v wayland-info >/dev/null 2>&1; then
    # `|| true`: with `pipefail` an empty grep would exit the script here,
    # before the BUG message below gets its say. The socket is read off the
    # "scoot is up" line only: Smithay's own startup lines also match
    # `wayland...".*"` (e.g. `smithay::wayland::output ... "headless"` sorts
    # earlier in the log), so an unanchored `head -1` grabs one of those and
    # the name extraction below comes up empty -- that misread failed the
    # whole script on any machine with wayland-info installed. The first
    # pattern tolerates the ANSI escapes tracing writes between a field name
    # and its value; the second then pulls the bare socket name out of that
    # match.
    wayland_socket=$(grep 'scoot is up' "$LOG" | grep -o 'wayland[^"]*"[^"]*"' | head -1 | grep -o 'wayland-[0-9]*' || true)
    if [ -z "$wayland_socket" ]; then
        echo "BUG: could not find the Wayland socket name in $LOG"
        tail -30 "$LOG"
        exit 1
    fi
    if ! WAYLAND_DISPLAY="$wayland_socket" wayland-info | grep -q 'zwp_linux_dmabuf_v1'; then
        echo "BUG: zwp_linux_dmabuf_v1 is not advertised -- the dmabuf global is missing"
        WAYLAND_DISPLAY="$wayland_socket" wayland-info | grep -c interface
        exit 1
    fi
    echo "ok: zwp_linux_dmabuf_v1 is advertised"
else
    echo "wayland-info not found -- skipping the dmabuf advertisement check"
fi

echo "--- checking foot no longer falls back to client-side decorations ---"
# Every prior smoke-test run on this project has shown foot logging this on
# startup (it's stderr, captured into $LOG along with everything else foot
# prints) because nothing ever answered zxdg_decoration_manager_v1 before
# this feature. Its absence now is the actual proof the protocol negotiation
# succeeded -- not just that `XdgDecorationState::new` didn't panic.
if grep -q "no decoration manager available" "$LOG"; then
    echo "BUG: foot is still falling back to client-side decorations -- zxdg_decoration_manager_v1 negotiation did not take effect"
    grep "no decoration manager available" "$LOG"
    exit 1
fi
echo "ok: foot's CSD-fallback warning is gone -- server-side decoration was negotiated"

echo "--- compositor log (tail) ---"
tail -20 "$LOG"

# The two scenarios below each start their own, separate compositor (a fresh
# --config only means anything at startup), always --headless -- config
# loading is backend-agnostic, so there's no need to repeat these under
# --nested/--tty too, the way the IPC-driven test above deliberately covers
# every backend.

echo "=== config file: a user bind actually takes effect ==="
run_config_bind_test() {
    local socket="$CONFIG_SOCK"
    local log="$CONFIG_LOG"
    local cfg
    cfg=$(mktemp "$CONFIG_TEMPLATE")
    # super+n is bound to nothing by default (see keybindings.rs) -- an
    # otherwise-unused combo, so this only passes if the config file's bind
    # is what moved focus, not some default binding coincidentally doing it.
    cat >"$cfg" <<'EOF'
[binds]
"super+n" = "focus-column right"
EOF
    rm -f "$socket" "$log"

    "$SCOOT" --headless --width 1200 --height 800 --socket "$socket" --config "$cfg" \
        >"$log" 2>&1 &
    # Not `local`: the EXIT trap below fires as this subshell itself exits,
    # which is after this function has already returned -- by which point a
    # `local` would already be out of scope, and `set -u` would fail the
    # trap itself with "pid: unbound variable", silently skipping the kill
    # it exists to do (this combination broke exactly that way during
    # development). A plain assignment lives for the rest of this subshell
    # (this function's only caller), exactly as long as the trap needs it.
    pid=$!
    # EXIT, not RETURN. This function's only caller wraps it as
    # `( run_config_bind_test ) || exit 1`, and a subshell on the left side
    # of `||` has errexit suppressed for everything it runs -- confirmed
    # empirically -- so an unguarded command failing below doesn't actually
    # abort the function early the way it would if this were called
    # directly. RETURN would therefore already fire correctly as written
    # here today. EXIT is used anyway, since it doesn't depend on that
    # wrapping detail staying exactly as written -- a future edit to the
    # call site could otherwise silently regress this cleanup with no
    # signal that it happened. Same pattern as the top-level $compositor
    # trap above.
    trap 'kill "$pid" 2>/dev/null || true' EXIT
    export SCOOT_SOCKET="$socket"

    for _ in $(seq 1 60); do
        [ -S "$socket" ] && break
        sleep 0.1
    done
    if [ ! -S "$socket" ]; then
        echo "config-bind test: the control socket never appeared; compositor log:"
        tail -20 "$log"
        return 1
    fi

    "$SCOOT" msg action spawn foot
    "$SCOOT" msg action spawn foot
    local mapped=0
    for _ in $(seq 1 100); do
        if [ "$("$SCOOT" msg windows | jq '.windows | length')" -eq 2 ]; then
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
    right_id=$("$SCOOT" msg windows | jq -r '.windows[] | select(.focused) | .id')
    left_id=$("$SCOOT" msg windows | jq -r --argjson id "$right_id" '.windows[] | select(.id != $id) | .id')

    # Move left with the default binding first, so the only way super+n can
    # bring focus back to the right column is if it really is bound to
    # focus-column right -- not e.g. a no-op at an already-rightmost column.
    "$SCOOT" msg key super+h
    "$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
    if [ "$(focused_window)" != "$left_id" ]; then
        echo "config-bind test: super+h did not move focus left; compositor log:"
        tail -30 "$log"
        return 1
    fi

    "$SCOOT" msg key super+n
    "$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
    if [ "$(focused_window)" != "$right_id" ]; then
        echo "BUG: the config file's super+n -> focus-column right bind did not take effect"
        "$SCOOT" msg windows
        return 1
    fi
    echo "ok: the config file's super+n bind (focus-column right) moved focus as configured"
}
( run_config_bind_test ) || exit 1

echo "=== config file: a capital-letter bind fires on the unshifted key ==="
run_capital_bind_test() {
    local socket="$CAPITAL_SOCK"
    local log="$CAPITAL_LOG"
    local cfg
    cfg=$(mktemp "$CAPITAL_TEMPLATE")
    # A lone "A" names the unshifted `a` key, not shift+a (see the [binds]
    # reference): injecting a bare `a` must close the window, and loading
    # the file must log the warning that says so.
    cat >"$cfg" <<'EOF'
[binds]
"A" = "close"
EOF
    rm -f "$socket" "$log"

    "$SCOOT" --headless --width 1200 --height 800 --socket "$socket" --config "$cfg" \
        >"$log" 2>&1 &
    # Not `local` -- and EXIT, not RETURN -- see run_config_bind_test's
    # identical trap for why.
    pid=$!
    trap 'kill "$pid" 2>/dev/null || true' EXIT
    export SCOOT_SOCKET="$socket"

    for _ in $(seq 1 60); do
        [ -S "$socket" ] && break
        sleep 0.1
    done
    if [ ! -S "$socket" ]; then
        echo "capital-bind test: the control socket never appeared; compositor log:"
        tail -20 "$log"
        return 1
    fi

    if ! grep -q "a single capital letter in \[binds\]" "$log"; then
        echo "BUG: no warning about the capital-letter bind -- the fold happened with no signal"
        tail -30 "$log"
        return 1
    fi
    echo "ok: the capital-letter fold was logged"

    "$SCOOT" msg action spawn foot
    local mapped=0
    for _ in $(seq 1 100); do
        if [ "$("$SCOOT" msg windows | jq '.windows | length')" -eq 1 ]; then
            mapped=1
            break
        fi
        sleep 0.2
    done
    if [ "$mapped" -ne 1 ]; then
        echo "capital-bind test: the window never mapped; compositor log:"
        tail -30 "$log"
        return 1
    fi

    "$SCOOT" msg key a
    "$SCOOT" msg wait-idle --quiet-ms 300 --timeout-ms 10000
    if [ "$("$SCOOT" msg windows | jq '.windows | length')" -ne 0 ]; then
        echo "BUG: the config file's \"A\" -> close bind did not fire on an unshifted 'a'"
        "$SCOOT" msg windows
        return 1
    fi
    echo "ok: the config file's \"A\" bind (close) fired on an unshifted 'a'"
}
( run_capital_bind_test ) || exit 1

echo "=== config file: a malformed file falls back to defaults instead of blocking startup ==="
run_broken_config_test() {
    local socket="$BROKEN_SOCK"
    local log="$BROKEN_LOG"
    local cfg
    cfg=$(mktemp "$BROKEN_TEMPLATE")
    printf 'this is not valid toml [[[\n' >"$cfg"
    rm -f "$socket" "$log"

    "$SCOOT" --headless --width 1200 --height 800 --socket "$socket" --config "$cfg" \
        >"$log" 2>&1 &
    # Not `local` -- and EXIT, not RETURN -- see run_config_bind_test's
    # identical trap for why.
    pid=$!
    trap 'kill "$pid" 2>/dev/null || true' EXIT
    export SCOOT_SOCKET="$socket"

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

    "$SCOOT" msg version
    "$SCOOT" msg action spawn foot
    local mapped=0
    for _ in $(seq 1 100); do
        if [ "$("$SCOOT" msg windows | jq '.windows | length')" -eq 1 ]; then
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
