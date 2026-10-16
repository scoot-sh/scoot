# The look leaves at runtime: the one helper per next-launch app every
# call site shares. A swap renders the look's own files under
# `<state>/look/` (`fuzzel.ini`, `swaylock`); these read them back at each
# start. Included raw (never shell-quoted) at the top of every script that
# needs one.
#
# Both apps replace their whole config when pointed at a file (`--config`
# for fuzzel, `-C` for swaylock, each skipping the default search), so
# neither is ever pointed at the look file: the user's own config stays
# the base and only the look's colors layer over it.

# `scoot_fuzzel_look_flags PATH` prints the look's seven menu colors (a
# rendered `fuzzel.ini` `[colors]` table, `rrggbbaa`) as fuzzel CLI flags,
# one line with one word per flag -- or nothing when PATH is absent or
# carries no usable color (callers then fall back to their eval-time
# flags, so a missing `awk` or a truncated look leaves the menu
# unthemed, not broken). The caller passes no `--config`, so fuzzel
# reads the user's own config as before; CLI flags win over the file, so
# the look's colors replace exactly the seven leaves the eval-time flags
# carried, while the font, the line count and every other key stay the
# user's. Unknown keys and malformed values never pass: a truncated look
# leaves the menu unthemed, not broken.
scoot_fuzzel_look_flags() {
    [ -f "$1" ] || return 0
    awk '
        /^\[colors\][ \t]*$/ { in_colors = 1; next }
        /^\[/ { in_colors = 0; next }
        in_colors && /=/ {
            key = $0; sub(/=.*$/, "", key)
            value = $0; sub(/^[^=]*=/, "", value)
            if ((key == "background" || key == "text" || key == "prompt" || key == "match" || key == "selection" || key == "selection-text" || key == "border") && value ~ /^[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]$/) {
                printf " --%s-color=%s", key, value
            }
        }
    ' "$1"
}

# `scoot_swaylock_merged LOOK SETTINGS` concatenates the live look's
# locker colors (first) with the user's own locker settings (after) into
# one private file and prints its path. swaylock reads duplicate keys
# last-wins, so user keys win per key while the look fills the rest;
# without this merge a swap would drop the image, the flags and every
# user color override behind the lock screen. The file lives under the
# runtime dir (per-user, cleared at logout) at one fixed name, replaced
# atomically, so concurrent locks never read a half-written file and no
# lock ever litters. Fails (nonzero, nothing printed) when the merge
# cannot be written: the caller then locks with its fallback config,
# because a lock that cannot theme must still lock.
scoot_swaylock_merged() {
    target="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/scoot-swaylock-config"
    tmp="$(mktemp "$target.XXXXXX")" || return 1
    if cat "$1" "$2" >"$tmp"; then
        mv -f "$tmp" "$target" && printf '%s' "$target"
    else
        rm -f "$tmp"
        return 1
    fi
}
