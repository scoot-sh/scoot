# scoot-bluetooth-pick: Bluetooth from the keyboard, through
# bluetoothctl and the launcher's dmenu contract (lines on stdin, the
# pick on stdout). The body of the script `apps-home.nix` builds, with
# every `@NAME@` replaced by an absolute tool path (or a bare name from
# PATH where the module knows no package; a missing tool then fails
# loud here). See site/src/content/docs/desktop/index.md#wifi-and-bluetooth.
#
#   list                    paired devices, `* MAC name` while
#                           connected, `  MAC name` otherwise (stdout
#                           is the picker's; all else stderr)
#   pick                    the paired devices plus "Pair a new
#                           device", the power switch and (with the
#                           audio slot) "Audio output" through the
#                           menu (the bind)
#   menu                    stdin's lines through the menu: the bar's
#                           `bluetooth.menu-command` (its rows are
#                           names, ` (connected)` where connected)
#   connect / disconnect X  one device, by MAC or name
#   pair X                  pair, trust and connect one device the
#                           last scan saw, by MAC or name
#   power on|off            the first controller's power
#
# A picked paired device toggles (connected disconnects, any other
# connects); a picked unpaired one pairs. Pairing runs with a
# no-input agent: what pairs with "just works" (headsets, mice, most
# keyboards and speakers) pairs from here; a device that insists on a
# typed PIN pairs from a terminal (`bluetoothctl`, then `pair MAC`).
#
# Every bluetoothctl call is bounded (`--timeout`): with no bluetoothd
# on the bus, bluetoothctl waits for one forever, so an unbounded call
# would hang the picker instead of failing. Every failure exits 1 and
# says why, on stderr and as a notification -- a picker spawned by a
# bind has no terminal.

BT=@BT@
MENU=@MENU@
SINK=@SINK@
NOTIFY=@NOTIFY@
AWK=@AWK@
TIMEOUT=@TIMEOUT@
# Whether the audio slot's sink helper is there to offer.
AUDIO=@AUDIO@
# The look's menu colors as fuzzel flags (empty without a look): split
# on purpose below, each flag one word.
THEME=@THEME@

PAIR_ROW="Pair a new device..."
AUDIO_ROW="Audio output..."
ON_ROW="Turn Bluetooth on"
OFF_ROW="Turn Bluetooth off"
# How long a pair scan listens, in seconds: long enough for a device in
# pairing mode to advertise, short enough to wait through.
SCAN_SECS=10

say() { printf 'scoot-bluetooth-pick: %s\n' "$*" >&2; }
notify() {
    "$TIMEOUT" 5 "$NOTIFY" --app-name=scoot Bluetooth "$*" >/dev/null 2>&1 || true
}
fail() {
    say "$*"
    notify "$*"
    exit 1
}

# A query (answers in milliseconds where BlueZ runs).
bt() { "$BT" --timeout 5 "$@"; }

# One action, its output kept for the failure message.
bt_act() {
    secs=$1
    shift
    if ! out=$("$BT" --timeout "$secs" "$@" 2>&1); then
        fail "$(printf '%s' "$out" | "$AWK" 'NF { line = $0 } END { print line ? line : "bluetoothctl gave up (BlueZ did not answer in time)" }')"
    fi
}

# Refuses (loud) unless BlueZ answers with a controller. Sets
# `powered` (yes/no) for the menu's power row.
require_ctl() {
    info=$(bt show 2>&1)
    rc=$?
    case "$info" in
        *"No default controller"*)
            fail "no Bluetooth controller (no hardware here, or it is blocked)"
            ;;
    esac
    if [ "$rc" -ne 0 ] || [ -z "$info" ]; then
        fail "BlueZ is not answering (is bluetooth.service running?)"
    fi
    powered=$(printf '%s\n' "$info" | "$AWK" '$1 == "Powered:" { print $2; exit }')
}

# `MAC<TAB>name` for each device a filter selects (`Paired`,
# `Connected`, or nothing for every device BlueZ knows).
devices() {
    bt devices "$@" 2>/dev/null | "$AWK" '
        $1 == "Device" && $2 ~ /^([0-9A-Fa-f][0-9A-Fa-f]:){5}[0-9A-Fa-f][0-9A-Fa-f]$/ {
            mac = $2
            name = $0
            sub(/^Device [^ ]+ ?/, "", name)
            print mac "\t" name
        }'
}

# The MAC for a MAC or a name, among `devices "$@"` (the first match).
resolve() {
    want=$1
    shift
    devices "$@" | WANT=$want "$AWK" -F'\t' '
        $1 == ENVIRON["WANT"] || substr($0, index($0, "\t") + 1) == ENVIRON["WANT"] { print $1; exit }'
}

is_paired() { [ -n "$(resolve "$1" Paired)" ]; }
is_connected() { [ -n "$(resolve "$1" Connected)" ]; }

paired_rows() {
    connected=$(devices Connected)
    devices Paired | CONNECTED=$connected "$AWK" -F'\t' '
        BEGIN { n = split(ENVIRON["CONNECTED"], rows, "\n"); for (i = 1; i <= n; i++) { split(rows[i], f, "\t"); on[f[1]] = 1 } }
        { printf "%s %s %s\n", ($1 in on) ? "*" : " ", $1, substr($0, index($0, "\t") + 1) }'
}

do_toggle() {
    mac=$1
    if is_connected "$mac"; then
        bt_act 15 disconnect "$mac"
    else
        bt_act 30 connect "$mac"
    fi
}

do_pair() {
    mac=$(resolve "$1")
    [ -n "$mac" ] || fail "no device '$1' in range (put it in pairing mode, then pick \"$PAIR_ROW\")"
    bt_act 60 --agent NoInputNoOutput pair "$mac"
    # Trusted, so it reconnects on its own next time.
    bt_act 10 trust "$mac"
    bt_act 30 connect "$mac"
}

# Listens for devices in pairing mode, then offers the new ones.
pair_pick() {
    notify "Looking for devices in pairing mode (${SCAN_SECS} s)..."
    # The scan runs until the bound; its exit status is the timeout's.
    "$BT" --timeout "$SCAN_SECS" scan on >/dev/null 2>&1 || true
    paired=$(devices Paired)
    rows=$(devices | PAIRED=$paired "$AWK" -F'\t' '
        BEGIN { n = split(ENVIRON["PAIRED"], rows, "\n"); for (i = 1; i <= n; i++) { split(rows[i], f, "\t"); old[f[1]] = 1 } }
        !($1 in old) { print $1 " " substr($0, index($0, "\t") + 1) }')
    [ -n "$rows" ] || fail "no new device found (is it in pairing mode?)"
    # shellcheck disable=SC2086
    sel=$(printf '%s\n' "$rows" | "$MENU" --dmenu --prompt='pair: ' --no-run-if-empty --only-match $THEME) || exit 0
    [ -n "$sel" ] || exit 0
    do_pair "${sel%% *}"
}

audio_pick() {
    sinks=$("$SINK" list) || fail "no audio outputs to offer (scoot-audio-sink list failed)"
    # shellcheck disable=SC2086
    choice=$(printf '%s\n' "$sinks" | "$MENU" --dmenu --prompt='sink: ' --no-run-if-empty --only-match $THEME) || exit 0
    [ -n "$choice" ] || exit 0
    exec "$SINK" set "${choice%% *}"
}

case "${1:-}" in
    list)
        require_ctl
        paired_rows
        ;;
    pick)
        require_ctl
        if [ "$powered" != "yes" ]; then
            rows=$ON_ROW
        else
            rows=$(paired_rows)
            rows=$(printf '%s\n%s\n' "$rows" "$PAIR_ROW")
            [ "$AUDIO" != 1 ] || rows=$(printf '%s\n%s\n' "$rows" "$AUDIO_ROW")
            rows=$(printf '%s\n%s\n' "$rows" "$OFF_ROW")
        fi
        # shellcheck disable=SC2086
        sel=$(printf '%s\n' "$rows" | "$AWK" 'NF' | "$MENU" --dmenu --prompt='bluetooth: ' --no-run-if-empty --only-match $THEME) || exit 0
        case "$sel" in
            "") exit 0 ;;
            "$ON_ROW") bt_act 10 power on ;;
            "$OFF_ROW") bt_act 10 power off ;;
            "$PAIR_ROW") pair_pick ;;
            "$AUDIO_ROW") audio_pick ;;
            *)
                # `* MAC name` or `  MAC name`: the MAC is the second field.
                mac=$(printf '%s\n' "$sel" | "$AWK" '{ print ($1 == "*") ? $2 : $1 }')
                do_toggle "$mac"
                ;;
        esac
        ;;
    menu)
        # The bar's rows are names, ` (connected)` where connected; a
        # paired one toggles, any other pairs.
        # shellcheck disable=SC2086
        sel=$("$MENU" --dmenu --prompt='bluetooth: ' --no-run-if-empty --only-match $THEME) || exit 0
        [ -n "$sel" ] || exit 0
        require_ctl
        name=${sel% (connected)}
        # A device with neither name nor alias shows in the bar as its
        # object path's last element (`dev_AA_BB_CC_DD_EE_FF`).
        case "$name" in
            dev_??_??_??_??_??_??)
                name=${name#dev_}
                name=${name//_/:}
                ;;
        esac
        mac=$(resolve "$name" Paired)
        if [ -n "$mac" ]; then
            do_toggle "$mac"
        else
            do_pair "$name"
        fi
        ;;
    connect | disconnect)
        [ -n "${2:-}" ] || { say "usage: scoot-bluetooth-pick $1 <mac-or-name>"; exit 2; }
        require_ctl
        mac=$(resolve "$2" Paired)
        [ -n "$mac" ] || fail "no paired device '$2' (pair it first: scoot-bluetooth-pick pair <mac-or-name>)"
        if [ "$1" = connect ]; then bt_act 30 connect "$mac"; else bt_act 15 disconnect "$mac"; fi
        ;;
    pair)
        [ -n "${2:-}" ] || { say "usage: scoot-bluetooth-pick pair <mac-or-name>"; exit 2; }
        require_ctl
        do_pair "$2"
        ;;
    power)
        case "${2:-}" in
            on | off)
                require_ctl
                bt_act 10 power "$2"
                ;;
            *)
                say "usage: scoot-bluetooth-pick power <on|off>"
                exit 2
                ;;
        esac
        ;;
    *)
        say "usage: scoot-bluetooth-pick {list|pick|menu|connect|disconnect|pair <mac-or-name>|power <on|off>}"
        exit 2
        ;;
esac
