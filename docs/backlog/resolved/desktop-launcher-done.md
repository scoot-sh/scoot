---
title: "Desktop launcher: fuzzel now, scootlaunch later"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Desktop launcher: fuzzel now, scootlaunch later

Filed 2026-10-04, child 4 of `desktop-paved-path`. Serves
**daily-drive** (no launcher, no desktop) and **computer use** (agents drive
`spawn` binds; a dmenu mode is the bar pickers' path too).

## The gap

`scootlaunch` is a pointer only (`docs/scootbar/backlog/launcher.md`: low,
M7, name settled `scootlaunch` with a `--dmenu` mode). The default binds
reference `wofi` (`docs/nix.md:308`) but the flake installs no launcher at
all — a fresh desktop's `ctrl+alt+space` spawns nothing.

## What to do

Fill the `desktop.launcher` slot with `fuzzel` (layer-shell `overlay`
native, no toolkit, fastest cold start of the maintained set — measure
against wofi/tofi/bemenu, record cold-start ms + closure, say why):

- Package + default binds (`ctrl+alt+space` drun plus a run mode; reconcile
  the existing `wofi` references in `docs/nix.md` / `home.nix:119` to the
  pick — no dangling default that spawns nothing).
- dmenu-mode contract now (lines on stdin, selection on stdout) matching
  the `scootlaunch --dmenu` shape in the pointer entry, so the bar's WiFi /
  power / audio pickers and a future scootlaunch swap unchanged.
- Recent-use ranking state file location (XDG state, say where).
- Edge cases: exclusive keyboard while open (compositor already supports
  it — `docs/protocols.md` keyboard focus); fullscreen interaction (overlay
  stays above); empty application list; sub-60 ms first frame budget —
  measure it.

Acceptance: eval pins in `nix/tests.nix` (binds point at the installed
launcher, no `wofi` default left); real-login proof on the M2 (open, type,
launch; dmenu round-trip from a pipe); docs in `docs/nix.md`.

## Not in this ticket

Building `scootlaunch` itself; file-picker dialogs (portal child);
emoji pickers.

## Resolution (2026-10-05, PR #451)

Landed as the `desktop.launcher` slot filled with fuzzel: `enable` +
`daemon` (`"fuzzel"`, widened later without renaming anything) in
`nix/modules/desktop.nix`, the package default in
`nix/modules/launcher-home.nix` (new) and `nix/modules/nixos.nix`,
one wrapper script (`scoot-launcher`: fuzzel by absolute store path,
`--layer=overlay`, the look's seven color flags shared with the
clipboard picker through `nix/modules/fuzzel-theme.nix` (new), extra
args passed through) beside the keymap's two binds in
`nix/modules/keys-home.nix` (`Super+d` drun, `Ctrl+Alt+Space` run via
`--list-executables-in-path`), on with the profile, per-target opt-out
`theme.targets.launcher.enable`. `wofi` reconciled everywhere
(`docs/nix.md`, `home.nix`, `docs/configuration.md`); `nix/tests.nix`
pins the binds at the installed script, the all-four-looks theming,
the refusals, and no `wofi` default in the modules or the rendered
binds.

Measured at the pinned rev (`8ce4ef6`, `aarch64-linux`): fuzzel
152.4 MiB full / 41.7 MiB over the profile (4 paths; 0 with the
clipboard, same derivation), wofi 342.3/67.4, tofi 113.0/0.2, bemenu
117.4/0.6; cold first frame 57/50/52 ms fuzzel (sub-60), 135/80/73
wofi, 62/41/46 tofi, 57/33/36 bemenu; RSS ~22 MB open, nothing when
closed. Proven live in a `scoot-test` login on the Asahi M2: both
binds' scripts open themed (screenshots in `docs/nix.md`), `foot`
typed and launched maps focused, dmenu pipe round-trips, locked
frames stay byte-identical with the spawn refused, empty app list
draws an empty menu. Full evidence in the implementer report.
