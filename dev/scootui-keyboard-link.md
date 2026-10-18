# `scootui-keyboard` and libxkbcommon: the link

`crates/scootui`'s `keyboard` feature links libxkbcommon dynamically
(`libxkbcommon.so.0` in `ldd`/`NEEDED`), through the `xkbcommon` crate
with `default-features = false` (its `wayland` feature is only the
memmap-based `new_from_fd`, which this crate does not use: the keymap fd
is read with an explicit size cap instead). This is the one C library in
the shell family's link set, accepted by the maintainer on 2026-10-17
(`dev/design/shell-apps.md`, Decisions needed 3; the measurement is
`dev/spikes/scootui-keyboard.md`).

## Where the library comes from

- **Builds and tests**: the dev shell (`nix/dev-shell.nix` via
  `vm/compositor-deps.nix`, which already lists `pkgs.libxkbcommon` for
  the compositor) and the dev VM's system closure. No Nix file changed
  for this ticket: the dependency was already in both places because
  scoot itself links it.
- **CI**: the scootui job runs in `nix develop`, so the same closure
  applies. The feature matrix covers `keyboard` alone and
  `keyboard,test-support` together (`.github/workflows/ci.yml`).
- **Future app packages** (`scootnotify`, `scootlaunch`, `scootgreet`
  when they enable the feature): each carries libxkbcommon in its Nix
  `buildInputs`, which sets RUNPATH. Nothing to do until then: no
  shipped binary enables the feature today.

## What does not link it

`scootbar` (and therefore `scootnotify`, which does not depend on
`scootui` at all) enables only `scootui/icons,wayland`: `cargo tree -p
scootbar -e normal` shows no `xkbcommon`, `ldd` shows no new library,
and the release binary is byte-identical in size and `NEEDED` to one
built without this ticket (see the PR evidence). CI's existing `ldd`
gate (`No libc crate, and what the release binary links`) fails the
build if that ever changes.

## When the library is missing

Loud, one message, no crash loop: the dynamic loader refuses the exec
with `libxkbcommon.so.0: cannot open shared object file` on stderr and
exit status 127. There is no fallback (keyboard text cannot work without
the decoder) and no in-process retry, so a supervisor cannot wedge the
session restarting it; units for keyboard-typing apps must not set
`Restart=always` across this failure (they do not exist yet -- this is
a constraint on their future unit files, recorded here so it is not
rediscovered).

## Release builds

The `x86_64`/`aarch64` release binaries that exist today (`scootbar`)
do not enable the feature and are unaffected. When an app package
enables it, its derivation's `buildInputs` (above) is what puts the
library in the closure; `ldd` on the result must show exactly one new
`NEEDED` entry, `libxkbcommon.so.0`.
