# Whether the session is unlocked, for the clipboard store entry and the
# history picker (both refuse while locked -- see
# site/src/content/docs/desktop/index.md#clipboard). A shell fragment, not a script: `clipboard-home.nix` and
# `keys-home.nix` embed it with `@SCOOT_BIN@` replaced by the scoot binary
# (absolute when the module knows one, bare `scoot` from PATH otherwise --
# the same substitution `nixos.nix` does for the session units).
#
# The probe is a `focus-window-id` for an id no window can hold
# (`u64::MAX`: window ids are never reused, so no session ever reaches
# it): unlocked it answers `ok` with no side effect, locked the
# compositor refuses it naming the lock. (Each probe spends an
# `on_demand` layer surface's keyboard focus -- a side-effect-free
# `locked` query on the wire would remove both the round trip and that;
# that query is compositor work in scoot, so the probe stays until the
# wire carries it.) A `version` first: it answers
# locked and unlocked alike, so it failing means IPC itself is down (no
# compositor yet, a files-only setup) -- fail open then and proceed, so
# a broken probe costs the lock guarantee (the wipe-on-lock and the
# suppressed binds still hold) rather than the history.
clipboard_unlocked() {
    if ! "@SCOOT_BIN@" msg version >/dev/null 2>&1; then
        return 0
    fi
    "@SCOOT_BIN@" msg action focus-window-id 18446744073709551615 >/dev/null 2>&1
}
