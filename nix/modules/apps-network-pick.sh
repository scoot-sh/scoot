# scoot-network-pick: WiFi from the keyboard, through nmcli and the
# launcher's dmenu contract (lines on stdin, the pick on stdout). The
# body of the script `apps-home.nix` builds, with every `@NAME@`
# replaced by an absolute tool path (or a bare name from PATH where
# the module knows no package; a missing tool then fails loud here).
# See site/src/content/docs/desktop/index.md#wifi-and-bluetooth.
#
#   list            candidate SSIDs, one per line: in range and saved
#                   first, then the rest in range, then saved but out
#                   of range (stdout is the picker's; all else stderr)
#   pick            `list` through the menu, then `connect` (the bind)
#   menu            stdin's lines through the menu, then `connect`
#                   (the bar's `network.menu-command`: its scan list)
#   connect SSID    join it (the bar's `network.connect-command`)
#
# Joining: a saved connection for the SSID goes up by UUID (no secret:
# NetworkManager kept it); an open one joins directly; a secured one
# takes its key from the keyring (`secret-tool lookup scoot-wifi SSID`)
# or else asks for it in a masked menu prompt, and the key travels to
# nmcli on stdin (`--ask`), never in an argument (the process table is
# readable by every user). A first join that fails (a wrong key) leaves
# no saved connection behind, so the next pick asks again; a keyring
# key the network refuses falls back to the prompt at once, so a stale
# keyring entry never loops. A network out of range, and an enterprise
# (802.1X) one, fail loud before any prompt: a key cannot join either.
#
# The join itself (nmcli, and the cleanup after a failed one) runs in a
# session of its own (`setsid --fork --wait`, so its status still comes
# back): the bar ends a menu it reopens, and a connect it replaces, by
# signalling the whole process group (SIGKILL 100 ms after SIGTERM),
# which would otherwise cut nmcli mid-join and skip the cleanup. The
# pick and the key prompt stay in the caller's group: reopening the
# bar's menu closes those, as it should. `_join` is that internal half
# (its key, when it has one, on stdin).
#
# Nothing here touches NetworkManager's own state beyond that: no
# rescans (the cached scan only: the bar's module scans), no radio
# switches. Every failure exits 1 and says why, on stderr and as a
# notification -- a picker spawned by a bind has no terminal.

NMCLI=@NMCLI@
MENU=@MENU@
SECRET=@SECRET@
NOTIFY=@NOTIFY@
AWK=@AWK@
TIMEOUT=@TIMEOUT@
SETSID=@SETSID@
# The look's menu colors as fuzzel flags (empty without a look): split
# on purpose below, each flag one word.
THEME=@THEME@

say() { printf 'scoot-network-pick: %s\n' "$*" >&2; }
fail() {
    say "$*"
    "$TIMEOUT" 5 "$NOTIFY" --app-name=scoot WiFi "$*" >/dev/null 2>&1 || true
    exit 1
}

# nmcli's terse output escapes `:` and `\` with a backslash: fields
# split on the unescaped colons only, unescaped as they split, so an
# SSID like "Hotel: Lobby" survives as one value.
AWK_LIB='
function unesc(s,   out, i, c) {
    out = ""
    for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (c == "\\" && i < length(s)) { i++; c = substr(s, i, 1) }
        out = out c
    }
    return out
}
function tsplit(line, f,   n, i, c, cur) {
    n = 1; cur = ""
    for (i = 1; i <= length(line); i++) {
        c = substr(line, i, 1)
        if (c == "\\" && i < length(line)) { i++; cur = cur substr(line, i, 1) }
        else if (c == ":") { f[n++] = cur; cur = "" }
        else cur = cur c
    }
    f[n] = cur
    return n
}
'

# Refuses (loud) unless NetworkManager answers and manages a WiFi
# device that is not unavailable (no hardware in a VM, a radio off).
require_wifi() {
    if ! devs=$("$NMCLI" -t -f DEVICE,TYPE,STATE device 2>&1); then
        fail "NetworkManager is not answering ($devs)"
    fi
    wifi=$(printf '%s\n' "$devs" | "$AWK" "$AWK_LIB"'
        { tsplit($0, f) }
        f[2] == "wifi" && f[3] != "unavailable" && f[3] != "unmanaged" { print f[1] }')
    [ -n "$wifi" ] || fail "no usable Wi-Fi device (no wireless hardware here, or its radio is off)"
}

# Saved WiFi connections as `UUID<TAB>SSID` (the SSID, not the
# connection's name: a profile named "Home" still matches its network).
saved_wifi() {
    uuids=$("$NMCLI" -t -f UUID,TYPE connection show 2>/dev/null | "$AWK" "$AWK_LIB"'
        { tsplit($0, f) }
        f[2] == "802-11-wireless" || f[2] == "wifi" { print f[1] }')
    [ -n "$uuids" ] || return 0
    set --
    for uuid in $uuids; do set -- "$@" uuid "$uuid"; done
    "$NMCLI" -t -f connection.uuid,802-11-wireless.ssid connection show "$@" 2>/dev/null | "$AWK" "$AWK_LIB"'
        index($0, "connection.uuid:") == 1 { uuid = substr($0, 17) }
        index($0, "802-11-wireless.ssid:") == 1 && uuid != "" {
            ssid = unesc(substr($0, 22))
            if (ssid != "") print uuid "\t" ssid
            uuid = ""
        }'
}

# The cached scan as `SECURITY<TAB>SSID`, one row per SSID (the first
# access point's security), hidden networks (no SSID) left out.
scan_wifi() {
    "$NMCLI" -t -f SSID,SECURITY device wifi list --rescan no 2>/dev/null | "$AWK" "$AWK_LIB"'
        { tsplit($0, f) }
        f[1] != "" && !seen[f[1]]++ { print f[2] "\t" f[1] }'
}

do_list() {
    require_wifi
    saved=$(saved_wifi)
    scan=$(scan_wifi)
    { printf '%s\n' "$saved" | "$AWK" -F'\t' 'NF >= 2 { print "S\t" substr($0, index($0, "\t") + 1) }'
      printf '%s\n' "$scan" | "$AWK" -F'\t' 'NF >= 2 { print "R\t" substr($0, index($0, "\t") + 1) }'
    } | "$AWK" '
        { tag = substr($0, 1, 1); ssid = substr($0, 3) }
        tag == "S" { if (!(ssid in saved)) { saved[ssid] = 1; sorder[++ns] = ssid } }
        tag == "R" { if (!(ssid in range)) { range[ssid] = 1; rorder[++nr] = ssid } }
        END {
            for (i = 1; i <= nr; i++) if (rorder[i] in saved) print rorder[i]
            for (i = 1; i <= nr; i++) if (!(rorder[i] in saved)) print rorder[i]
            for (i = 1; i <= ns; i++) if (!(sorder[i] in range)) print sorder[i]
        }'
}

# One nmcli action, its output kept for the failure message (stdin
# passes through: the key, for `--ask`).
nm_act() {
    if ! out=$("$NMCLI" "$@" 2>&1); then
        fail "$(printf '%s' "$out" | "$AWK" 'NF { line = $0 } END { print line }')"
    fi
}

# The masked key prompt (`$2` is the prompt's text); an empty or
# cancelled one exits 0, joining nothing.
ask_key() {
    # shellcheck disable=SC2086
    key=$("$MENU" --dmenu --password --prompt-only="$2" $THEME </dev/null) || exit 0
    [ -n "$key" ] || exit 0
    printf '%s\n' "$key"
}

do_connect() {
    ssid=$1
    if [ -z "$ssid" ]; then
        say "usage: scoot-network-pick connect <ssid>"
        exit 2
    fi
    require_wifi
    uuid=$(saved_wifi | WANT=$ssid "$AWK" -F'\t' '
        substr($0, index($0, "\t") + 1) == ENVIRON["WANT"] { print $1; exit }')
    if [ -n "$uuid" ]; then
        join up "$uuid" </dev/null
        exit $?
    fi
    sec=$(scan_wifi | WANT=$ssid "$AWK" -F'\t' '
        substr($0, index($0, "\t") + 1) == ENVIRON["WANT"] { print "seen:" $1; exit }')
    case "$sec" in
        "")
            # Never prompt for a key to a network nobody can see (a
            # hidden one joins from a terminal: nmcli's `hidden yes`).
            fail "'$ssid' is not in range (not in the last scan, and not saved)"
            ;;
        seen: | seen:-- | seen:OWE*)
            join open "$ssid" </dev/null
            exit $?
            ;;
        *802.1X*)
            fail "'$ssid' uses enterprise (802.1X) login, which a key cannot join: set it up with nmtui"
            ;;
    esac
    # Secured: the keyring first, then a masked prompt.
    psk=$("$TIMEOUT" 30 "$SECRET" lookup scoot-wifi "$ssid" 2>/dev/null) || psk=""
    if [ -z "$psk" ]; then
        src=prompt
        psk=$(ask_key "$ssid" "key for $ssid: ") || exit 0
        [ -n "$psk" ] || exit 0
    else
        src=keyring
    fi
    printf '%s\n' "$psk" | join "$src" "$ssid"
    exit $?
}

# Runs `_join ARGS...` in its own session, returning its status (see
# the header).
join() {
    "$SETSID" --fork --wait "$0" _join "$@"
}

# One secured join with `$2`'s key, which reaches nmcli on its stdin
# (never an argument, never the environment): `out` holds nmcli's
# output.
# A failed one removes the profile it created (nmcli saves one before
# it knows the key works, and a saved profile is joined by UUID from
# then on: left behind, the wrong key would be retried forever). Every
# saved profile from before the join is spared (nmcli reuses one it
# finds for the network, and that one is the user's to keep); with no
# snapshot at all, nothing is removed.
join_key() {
    before=$("$NMCLI" -t -f UUID connection show 2>/dev/null)
    snap=$?
    if out=$(printf '%s\n' "$2" | "$NMCLI" --ask device wifi connect "$1" 2>&1); then
        return 0
    fi
    if [ "$snap" -eq 0 ]; then
        saved_wifi | WANT=$1 BEFORE=$before "$AWK" -F'\t' '
            BEGIN { n = split(ENVIRON["BEFORE"], b, "\n"); for (i = 1; i <= n; i++) old[b[i]] = 1 }
            substr($0, index($0, "\t") + 1) == ENVIRON["WANT"] && !($1 in old) { print $1 }' |
            while IFS= read -r stale; do
                "$NMCLI" connection delete uuid "$stale" >/dev/null 2>&1 || true
            done
    fi
    return 1
}

# nmcli's last word, for the failure message.
last_line() { printf '%s' "$1" | "$AWK" 'NF { line = $0 } END { print line }'; }

# The join half, in its own session:
#   _join up UUID          a saved network, by its profile
#   _join open SSID        an open one
#   _join keyring SSID     a secured one, the keyring's key on stdin:
#                          refused for its secrets, it falls back to
#                          the prompt once
#   _join prompt SSID      a secured one, the typed key on stdin
do_join() {
    case "$1" in
        up) nm_act connection up uuid "$2" ;;
        open) nm_act device wifi connect "$2" ;;
        keyring | prompt)
            IFS= read -r key || [ -n "$key" ] || fail "no key for '$2'"
            if join_key "$2" "$key"; then exit 0; fi
            case "$1:$out" in
                keyring:*"Secrets were required"*)
                    key=$(ask_key "$2" "key for $2 (the keyring's was refused): ") || exit 0
                    [ -n "$key" ] || exit 0
                    join_key "$2" "$key" && exit 0
                    ;;
            esac
            fail "$(last_line "$out")"
            ;;
        *)
            say "usage: scoot-network-pick _join {up|open|keyring|prompt} <target>"
            exit 2
            ;;
    esac
}

# The menu half: stdin's lines through the menu, the pick into
# `connect`. A cancel (Escape, or an empty feed with
# `--no-run-if-empty`) joins nothing and exits 0.
menu_pick() {
    # shellcheck disable=SC2086
    sel=$("$MENU" --dmenu --prompt='wifi: ' --no-run-if-empty --only-match $THEME) || exit 0
    [ -n "$sel" ] || exit 0
    do_connect "$sel"
}

case "${1:-}" in
    list) do_list ;;
    pick)
        rows=$(do_list) || exit 1
        [ -n "$rows" ] || fail "no networks in range (none in the cached scan, and none saved)"
        printf '%s\n' "$rows" | menu_pick
        ;;
    menu) menu_pick ;;
    connect) do_connect "${2:-}" ;;
    _join) do_join "${2:-}" "${3:-}" ;;
    *)
        say "usage: scoot-network-pick {list|pick|menu|connect <ssid>}"
        exit 2
        ;;
esac
