# The locker's live pictures at runtime (bash only: arrays, `+=`,
# `$'\t'`, `${var//pat/rep}`), read raw into the `scoot-locker` script
# alone (`idle-home.nix`) -- not into `look-leaf.sh`, which other scripts
# include and which stays plain POSIX.
# `scoot_lockscreen_images` sets the array `lock_images` to one
# `-i NAME:FILE` pair of swaylock arguments per output scootbg keeps a
# blurred picture of the wallpaper for, read from its index at lock time
# (`$XDG_RUNTIME_DIR/scootbg-<display>.lockscreen/index`, one
# `name<TAB>file` line per output the daemon is showing now: an
# unplugged output's line is gone, a `scootbg set` on one output changes
# only its line's file). The array is empty whenever there is nothing
# usable -- no scootbg running, no index, a line that is not what scootbg
# writes -- and the caller then locks with the config's own `image`, and
# below that its `color`: a lock that cannot theme must still lock.
#
# The display name is scootbg's (`paths::display_name`): the last
# component of `$WAYLAND_DISPLAY` (`wayland-0` when unset), each byte
# outside `[A-Za-z0-9._-]` replaced by `_`, byte by byte (the C locale
# makes the pattern bytes). A file path must be plain `/`, letters,
# digits, `.`, `_` and `-`: swaylock passes an image path through
# `wordexp`, so anything else could expand into something else. Always
# returns success.
scoot_lockscreen_images() {
    lock_images=()
    local LC_ALL=C display index name file
    [ -n "${XDG_RUNTIME_DIR:-}" ] || return 0
    display=${WAYLAND_DISPLAY:-wayland-0}
    display=${display##*/}
    [ -n "$display" ] || return 0
    display=${display//[!A-Za-z0-9._-]/_}
    index="$XDG_RUNTIME_DIR/scootbg-$display.lockscreen/index"
    [ -r "$index" ] || return 0
    while IFS=$'\t' read -r name file; do
        case "$name" in "" | *:*) continue ;; esac
        case "$file" in /*) ;; *) continue ;; esac
        case "$file" in *[!A-Za-z0-9/._-]*) continue ;; esac
        [ -f "$file" ] || continue
        lock_images+=(-i "$name:$file")
    done <"$index"
    return 0
}
