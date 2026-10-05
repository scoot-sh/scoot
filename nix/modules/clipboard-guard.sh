# Whether the session is unlocked, for the clipboard store entry and the
# history picker (both refuse while locked -- see
# site/src/content/docs/desktop/index.md#clipboard). A shell fragment, not a script: `clipboard-home.nix` and
# `keys-home.nix` embed it with `@SCOOT_BIN@` replaced by the scoot binary
# (absolute when the module knows one, bare `scoot` from PATH otherwise --
# the same substitution `nixos.nix` does for the session units).
#
# The probe is the side-effect-free `locked` query: unlocked it answers
# `{"type":"locked","locked":false}`, locked `..."locked":true`. (It
# replaces the old `focus-window-id u64::MAX` probe, which spent an
# `on_demand` layer surface's keyboard focus -- an open bar dropdown's --
# on every copy.) A failing query means IPC itself is down (no compositor
# yet, a files-only setup, or a server predating the query, which answers
# it with an error) -- fail open then and proceed, so a broken probe
# costs the lock guarantee (the wipe-on-lock and the suppressed binds
# still hold) rather than the history.
clipboard_unlocked() {
    probe="$("@SCOOT_BIN@" msg locked 2>/dev/null)" || return 0
    # `scootctl` prints replies pretty (`"locked": true`) while the event
    # stream and any compact reader use `"locked":true`: match both.
    case "$probe" in
        *'"locked":true'*|*'"locked": true'*) return 1 ;;
        *) return 0 ;;
    esac
}
