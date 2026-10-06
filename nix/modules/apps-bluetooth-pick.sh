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
#   pick                    a state row (picking it does nothing),
#                           the paired devices, "Pair a new device",
#                           (with the audio slot) "Audio output" and
#                           the power switch through the menu (the
#                           bind, and the bar's click)
#   menu                    stdin's lines through the menu: the bar's
#                           `bluetooth.menu-command` (its rows are
#                           names, ` (connected)` where connected)
#   connect / disconnect X  one device, by MAC or name
#   pair X                  pair, trust and connect one device the
#                           last scan saw, by MAC or name
#   power on|off            the first controller's power
#
# A picked paired device toggles (connected disconnects, any other
# connects); a picked unpaired one pairs. A name is the device's
# alias, the name `bluetoothctl devices` lists (BlueZ's `Alias`: the
# device's own name unless renamed), or else its own `Name` (what the
# bar shows); two devices answering to one name are refused, never
# guessed between. Pairing registers no agent of its own
# (bluetoothctl registers none for a one-shot command): BlueZ then
# pairs as NoInputNoOutput, or through the session's default agent
# where one runs (blueman's, say). What pairs with "just works"
# (headsets, mice, most keyboards and speakers) pairs from here; a
# device that insists on a typed PIN pairs from a terminal
# (`bluetoothctl`, then `pair MAC`).
#
# Every bluetoothctl call runs under coreutils' `timeout`, never under
# bluetoothctl's own `--timeout`: with that flag a finished command
# never quits (bluez 5.87 `bt_shell_noninteractive_quit`), so every
# call took the full N seconds and exited 0 whatever happened. Under
# `timeout` an answered call returns at once with its real status,
# and one BlueZ never answers (no bluetoothd on the bus: bluetoothctl
# waits for one forever) ends at the bound. Only `scan on` keeps
# `--timeout`, where listening for the full time is the point. stdin
# is /dev/null throughout: a `connect` of a device BlueZ no longer
# holds asks "Scan and connect (yes/no)" on it. Every failure exits 1
# and says why, on stderr and as a notification -- a picker spawned by
# a bind has no terminal.

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
# The bound on a query (`show`, `devices`, `info`): BlueZ answers those
# in milliseconds, so this only ever ends a BlueZ that is not there.
QUERY_SECS=5

say() { printf 'scoot-bluetooth-pick: %s\n' "$*" >&2; }
notify() {
    "$TIMEOUT" 5 "$NOTIFY" --app-name=scoot Bluetooth "$*" >/dev/null 2>&1 || true
}
fail() {
    say "$*"
    notify "$*"
    exit 1
}

# One bluetoothctl call, bounded: `bt_run SECS ARGS...`. Past the bound
# it is sent SIGTERM (and SIGKILL 2 s later), and the status is
# `timeout`'s 124 (137 after the SIGKILL).
bt_run() {
    secs=$1
    shift
    "$TIMEOUT" -k 2 "$secs" "$BT" "$@" </dev/null
}

# Whether a status is the bound's, not bluetoothctl's own.
timed_out() { [ "$1" -eq 124 ] || [ "$1" -eq 137 ]; }

# The reason in a failed call's output: its `Failed to ...` (or
# `... not available`) line, else its last line, with the colors and
# the asynchronous `[CHG]`/`[NEW]`/`[DEL]` event lines that interleave
# with it left out.
reason() {
    printf '%s\n' "$1" | "$AWK" '
        { gsub(/\033\[[0-9;]*[A-Za-z]/, ""); gsub(/[\001\002\r]/, "") }
        /^\[(CHG|NEW|DEL)\]/ { next }
        NF { last = $0 }
        /Failed to|not available|No default controller/ { why = $0 }
        END { print (why != "") ? why : (last != "") ? last : "bluetoothctl failed with no message" }'
}

# A query that ran out its bound, named.
no_answer() {
    fail "BlueZ is not answering (no reply within $QUERY_SECS s: is bluetooth.service running?)"
}

# A query, its output on stdout; a failure is loud (and, inside a
# command substitution, exits that subshell 1 for the caller's
# `|| exit 1`).
bt() {
    out=$(bt_run "$QUERY_SECS" "$@" 2>&1)
    rc=$?
    if [ "$rc" -ne 0 ]; then
        timed_out "$rc" && no_answer
        fail "$(reason "$out")"
    fi
    printf '%s\n' "$out"
}

# One action (`bt_act SECS ARGS...`): its real status, its reason on a
# failure, and the bound named when it ran out.
bt_act() {
    secs=$1
    shift
    out=$(bt_run "$secs" "$@" 2>&1)
    rc=$?
    if [ "$rc" -ne 0 ]; then
        timed_out "$rc" && fail "$1 got no answer within $secs s (out of range, or BlueZ stopped answering)"
        fail "$(reason "$out")"
    fi
}

# Refuses (loud) unless BlueZ answers with a controller. Sets
# `powered` (yes/no) for the menu's power row.
require_ctl() {
    info=$(bt_run "$QUERY_SECS" show 2>&1)
    rc=$?
    case "$info" in
        *"No default controller"*)
            fail "no Bluetooth controller (no hardware here, or it is blocked)"
            ;;
    esac
    timed_out "$rc" && no_answer
    [ -n "$info" ] || fail "BlueZ is not answering (is bluetooth.service running?)"
    [ "$rc" -eq 0 ] || fail "$(reason "$info")"
    powered=$(printf '%s\n' "$info" | "$AWK" '$1 == "Powered:" { print $2; exit }')
}

# `MAC<TAB>alias` for each device a filter selects (`Paired`,
# `Connected`, or nothing for every device BlueZ knows).
#
# bluetoothctl prints a device in bold gray, `ESC[1;30mDevice MAC
# alias ESC[0m` -- colors and all, even into a pipe -- when its last
# advertisement carried neither discoverable flag: advertising, but
# not in pairing mode (bluez 5.87 `print_device`). BlueZ keeps those
# flags on the device, so paired earbuds or a mouse can stay gray for
# good: every list strips the colors and keeps the row. Only the pair
# list leaves gray rows out (`devices --discoverable`, `pair_pick`'s
# alone), on purpose: a device not in pairing mode will not pair from
# here, and every neighbour's beacon and earbuds would flood it. A
# device named outright (`pair X`, the bar's row) still resolves gray
# or not: that pick is the user's, and BlueZ's refusal says why.
devices() {
    gray=1
    if [ "${1:-}" = --discoverable ]; then
        gray=0
        shift
    fi
    rows=$(bt devices "$@") || exit 1
    printf '%s\n' "$rows" | "$AWK" -v gray="$gray" '
        {
            dim = index($0, "\033[1;30mDevice ") == 1
            gsub(/\033\[[0-9;]*[A-Za-z]/, "")
        }
        dim && !gray { next }
        $1 == "Device" && $2 ~ /^([0-9A-Fa-f][0-9A-Fa-f]:){5}[0-9A-Fa-f][0-9A-Fa-f]$/ {
            mac = $2
            name = $0
            sub(/^Device [^ ]+ ?/, "", name)
            print mac "\t" name
        }'
}

# The MAC for a MAC or a name, among `devices "$@"`: by MAC or alias
# first, then by each device's own `Name` (one `info` each, only when
# no alias matched: the bar shows `Name`, which a rename leaves
# behind). Nothing for no match; loud for two.
resolve() {
    want=$1
    shift
    rows=$(devices "$@") || exit 1
    hits=$(printf '%s\n' "$rows" | WANT=$want "$AWK" -F'\t' '
        NF && ($1 == ENVIRON["WANT"] || substr($0, index($0, "\t") + 1) == ENVIRON["WANT"]) { print $1 }')
    case "$want" in
        ??:??:??:??:??:??) ;;
        *) [ -n "$hits" ] || hits=$(by_name "$want" "$rows") || exit 1 ;;
    esac
    case "$hits" in
        *"
"*) fail "more than one device is named '$want': pick it by address (Super+b lists them)" ;;
    esac
    printf '%s\n' "$hits"
}

# The MACs among `rows` (as `devices` prints them) whose own `Name` is
# `$1`: one bounded `info` each. A device whose `info` fails is not
# that name, and is skipped: a temporary device BlueZ drops between
# `devices` and here (30 s unseen) answers "not available". A BlueZ
# that stops answering still fails loud, at the first bound.
by_name() {
    for mac in $(printf '%s\n' "$2" | "$AWK" -F'\t' 'NF { print $1 }'); do
        card=$(bt_run "$QUERY_SECS" info "$mac" 2>&1)
        rc=$?
        if [ "$rc" -ne 0 ]; then
            timed_out "$rc" && no_answer
            continue
        fi
        name=$(printf '%s\n' "$card" | "$AWK" '$1 == "Name:" { sub(/^[ \t]*Name: /, ""); print; exit }')
        [ "$name" != "$1" ] || printf '%s\n' "$mac"
    done
}

paired_rows() {
    connected=$(devices Connected) || exit 1
    paired=$(devices Paired) || exit 1
    printf '%s\n' "$paired" | CONNECTED=$connected "$AWK" -F'\t' '
        BEGIN { n = split(ENVIRON["CONNECTED"], rows, "\n"); for (i = 1; i <= n; i++) { split(rows[i], f, "\t"); on[f[1]] = 1 } }
        NF { printf "%s %s %s\n", ($1 in on) ? "*" : " ", $1, substr($0, index($0, "\t") + 1) }'
}

do_toggle() {
    mac=$1
    on=$(resolve "$mac" Connected) || exit 1
    if [ -n "$on" ]; then
        bt_act 15 disconnect "$mac"
    else
        bt_act 30 connect "$mac"
    fi
}

do_pair() {
    mac=$(resolve "$1") || exit 1
    [ -n "$mac" ] || fail "no device '$1' in range (put it in pairing mode, then pick \"$PAIR_ROW\")"
    bt_act 60 pair "$mac"
    # Trusted, so it reconnects on its own next time.
    bt_act 10 trust "$mac"
    bt_act 30 connect "$mac"
}

# Listens for devices in pairing mode, then offers the new ones.
pair_pick() {
    notify "Looking for devices in pairing mode (${SCAN_SECS} s)..."
    # The one `--timeout`: discovery runs until it fires (exiting 0),
    # and ends with the process. The outer bound covers a BlueZ that
    # never answers at all.
    scan=$("$TIMEOUT" -k 2 $((SCAN_SECS + 5)) "$BT" --timeout "$SCAN_SECS" scan on </dev/null 2>&1)
    case "$scan" in
        *"Failed to start discovery"*) fail "$(reason "$scan")" ;;
    esac
    paired=$(devices Paired) || exit 1
    # Only devices in pairing mode (see `devices`).
    all=$(devices --discoverable) || exit 1
    rows=$(printf '%s\n' "$all" | PAIRED=$paired "$AWK" -F'\t' '
        BEGIN { n = split(ENVIRON["PAIRED"], rows, "\n"); for (i = 1; i <= n; i++) { split(rows[i], f, "\t"); old[f[1]] = 1 } }
        NF && !($1 in old) { print $1 " " substr($0, index($0, "\t") + 1) }')
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
        head=""
        if [ "$powered" != "yes" ]; then
            rows=$ON_ROW
        else
            rows=$(paired_rows) || exit 1
            # The first row is the state, and picking it does nothing:
            # the menu opens on it, so a stray Enter toggles no device
            # (a connected Bluetooth keyboard would drop).
            head="Bluetooth is on ($(printf '%s\n' "$rows" | "$AWK" '$1 == "*" { n++ } END { print n + 0 }') connected)"
            rows=$(printf '%s\n%s\n%s\n' "$head" "$rows" "$PAIR_ROW")
            [ "$AUDIO" != 1 ] || rows=$(printf '%s\n%s\n' "$rows" "$AUDIO_ROW")
            rows=$(printf '%s\n%s\n' "$rows" "$OFF_ROW")
        fi
        # shellcheck disable=SC2086
        sel=$(printf '%s\n' "$rows" | "$AWK" 'NF' | "$MENU" --dmenu --prompt='bluetooth: ' --no-run-if-empty --only-match $THEME) || exit 0
        case "$sel" in
            "" | "$head") exit 0 ;;
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
        mac=$(resolve "$name" Paired) || exit 1
        if [ -n "$mac" ]; then
            do_toggle "$mac"
        else
            do_pair "$name"
        fi
        ;;
    connect | disconnect)
        [ -n "${2:-}" ] || { say "usage: scoot-bluetooth-pick $1 <mac-or-name>"; exit 2; }
        require_ctl
        mac=$(resolve "$2" Paired) || exit 1
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
