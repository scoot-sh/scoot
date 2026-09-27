---
title: "Seamless in scoot: a [wallpaper] config section"
status: "open"
area: "scootbg"
priority: "high"
blocked: "needs scootbg's daemon and set command first"
---

# Seamless in scoot: a [wallpaper] config section

In scoot, the wallpaper should be one config section, with nothing else to
wire up.

```toml
# Planned. No scoot release accepts this yet: scoot's config rejects
# unknown sections and, when it does, ignores the WHOLE file (your binds
# and layout included). Do not add it until this item lands.
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```

## The rule: whichever you changed last wins

Stated once here, and the same way in the README and in
[restore-state.md](restore-state.md):

- Edit `[wallpaper]` (and start or reload scoot): the config's wallpaper
  shows.
- Run `scootbg set` afterwards: that wallpaper shows, and keeps showing
  across restarts and unrelated reloads, until you next change
  `[wallpaper]` itself.

### Mechanism: one command for everything from the config

`apply-config` belongs to this ticket: it moved here from the CLI ticket
([cli-and-ipc-done.md](resolved/cli-and-ipc-done.md#resolution)) when
that ticket's other items landed, because it needs ticket 9's saved state
([restore-state.md](restore-state.md)) for its fingerprint. Nothing of it
exists yet; the CLI neither parses nor advertises it.

Everything scoot does with scootbg goes through one command,
`scootbg apply-config --profile NAME '<json>'`, where the JSON is the
`[wallpaper]` section's wallpaper values (`{}` when the section is
absent). scoot never uses `set`, `clear` or `daemon` itself.
`apply-config`:

1. Computes a fingerprint of the values, over a **canonical encoding**:
   scootbg parses the JSON and re-encodes it with sorted keys before
   hashing, never trusting the sender's byte order (a `HashMap` of
   per-output tables serializes in a different order every process). The
   values are taken after scoot resolves paths, and **`command` is not
   among them**: it is scoot's own key for finding the binary, and with
   the home-manager module it is a store path that changes on every
   upgrade, which would otherwise re-apply the config and wipe a
   `scootbg set` pick on each one.
2. If a daemon answers on the socket, hands it the section. If none does,
   becomes the daemon itself with the section as its starting point. Two
   racing starts (scoot's and an `[autostart]` entry's, say) are settled
   by the socket bind: the loser forwards to the winner, so the config's
   values are never dropped. A daemon started by `apply-config` uses its
   `--profile`; a forwarded `apply-config` carries its profile, and **the
   daemon adopts each one**: from then on it compares,
   restores and records (later `scootbg set`s included) in that profile's
   state, whatever profile it started with. The config owns the profile,
   so which process won the bind never changes where state lives.
3. The daemon compares the fingerprint with the one in that profile's
   state, which records the last section applied *from the config*:
   - **Different:** the section changed since it was last applied (or
     was never applied). Apply it (an empty section means clear) and
     record the new fingerprint.
   - **Same:** the section has not changed. Keep what is showing, or
     (at startup, or on adopting a profile) restore that profile's saved
     state, which includes any later `scootbg set`.

That delivers the rule in every order, because every config-origin
change records its fingerprint and nothing else does:

| Sequence | Result |
|---|---|
| section A, `set X`, restart | A unchanged, so X is restored |
| section A, reload with B, `set X`, restart | B unchanged since its reload, so X |
| section A, reload with B, restart | B shows |
| section removed by reload, later re-added as A | `{}` was recorded at removal, so A differs and shows |
| no daemon yet, section added by reload | `apply-config` starts the daemon |
| an `[autostart]` entry also starts `scootbg daemon` | whichever binds first, the daemon ends up on scoot's profile, so a `set X` from a previous boot is restored either way |

One order it cannot see: the section removed *while scoot is not
running* and re-added unchanged before the next start. No `{}` was ever
applied, so the old fingerprint matches and the last saved state (which
may be a `scootbg set` pick) restores. The config did not change between
the two runs that scoot saw, so this is still "unchanged", and it is
documented rather than worked around.

## How scoot drives it

- **Startup, and every reload, while `[wallpaper]` exists:** spawn
  `scootbg apply-config`, alongside `[autostart]`. No autostart entry is
  needed. Spawning on every reload, not only when the section changed,
  costs one short process and is a no-op when nothing changed (the
  fingerprint matches), and it means `scootctl reload` brings back a
  daemon that crashed, since scoot does not supervise it. Spawn order
  does not decide which client commits first, so nothing here promises
  the wallpaper appears before a bar.
- **No section at startup: scoot spawns nothing.** A user without
  scootbg installed never sees a missing-binary warning, and a user who
  runs `scootbg daemon` from `[autostart]` without a section keeps their
  `scootbg set` pick (a first `apply-config '{}'` would count as "never
  applied" and clear it).
- **Removing the section** is a reload with `{}`: the wallpaper clears,
  and the daemon keeps running (so a later `scootbg set` still works).
  `apply-config '{}'` with no daemon running records both the fingerprint
  and the clear in that profile's state, then exits, rather than starting
  a daemon only to clear; a later `scootbg daemon --profile scoot` shows
  nothing, as the config asked.
- **The profile** names the state `apply-config` restores and records
  (see [restore-state.md](restore-state.md)): scoot passes `scoot`, or
  `scoot-nested` under `--nested`, so a nested session and its host keep
  separate state.
- **Paths are resolved by scoot**, which owns its config's meaning: `~/`
  expanded and a relative path taken against the config file's directory,
  before encoding. scoot spawns without a shell, so nothing else expands
  them.
- **Never wait on scootbg from the event loop.** `scootbg set` only
  finishes once scoot has processed its Wayland commit, so scoot waiting
  for it on its own loop thread would deadlock. Every scootbg invocation is
  spawned and left to the existing SIGCHLD reaper; if a failed status
  should be logged, the reaper gains that ability (today it discards
  statuses, `child_reaper.rs`), still without blocking.
- **One-way coupling.** scoot depends on scootbg's command line, never its
  crate, and scootbg never reads scoot's config. The scoot side lives in
  the compositor (config parsing, spawn, reload), not in `scoot-core`,
  which stays platform-independent.
- **Failure is loud but harmless.** A missing `scootbg` binary, a bad path
  or a decode error logs a warning and leaves `background_color` showing.
  The session always starts, as with `[autostart]`.
- **The gap before the first frame.** Until scootbg commits its first
  buffer, scoot shows `background_color`. Measure that gap on `--tty`. If
  it is visible, the goal is to shorten it (commit sooner, decode less),
  not to paint a different placeholder color, which would be its own
  flash. A redundant `scootbg daemon` in `[autostart]` that wins the bind
  first restores the `default` profile, then adopts scoot's, which can
  flash an old wallpaper; the `[wallpaper]` docs tell users to drop that
  entry.

## Finding the binary (packaging)

The `scoot` package deliberately contains only `scoot` (`scootctl` is its
own package, flake.nix, gh #172), so `scootbg` is its own package too, and
scoot must be able to find it:

- scoot runs `scootbg` from `PATH` by default, with `[wallpaper] command`
  to point elsewhere (an absolute path).
- The home-manager module installs the `scootbg` package whenever its
  rendered settings contain `wallpaper`, and sets `command` to that
  package's store path, so a Nix user gets it working with no extra step.
- A missing binary is the fail-open warning above, naming the `command`
  it tried and how to install it.

### NixOS consumers

(User, 2026-09-27: "make sure ticket 10 enhances the nix setup for NixOS
consumers", and "it should flow well with scoot".)

**The bar: turning scoot on is enough.** A NixOS user who writes
`programs.scoot.enable = true` and puts a `[wallpaper]` section in their
config gets a wallpaper. They add no second option, no package, and no
`command` path. scootbg is a detail of scoot there, not a second thing to
set up. The same holds for home-manager. Both modules spell the options
the same way, under `programs.scoot.wallpaper.*`, so one reads like the
other.

Today the NixOS module (`nix/modules/nixos.nix`, `nixosModules.scoot`)
owns the scoot binary and the opt-in session entry, and nothing of
scootbg. After this item, a NixOS user must get a working `[wallpaper]`
with no hand wiring. That includes a user without home-manager, and one
who launches scoot from the greeter entry.

- **`programs.scoot.wallpaper.enable`** installs
  **`programs.scoot.wallpaper.package`** system-wide, so the default
  `[wallpaper] command = "scootbg"` resolves on `PATH` for any user and
  for the greeter-launched session.
  - **It defaults to `programs.scoot.enable`.** Installing a ~1.5 MB
    binary changes nothing until a config asks for a wallpaper, so this is
    not a behaviour change, unlike the login entry, which stays opt-in.
  - Setting it to `false` opts out, for someone using another wallpaper
    daemon.
- **One pinned pair.** scoot and scootbg come from the same flake
  revision by default, so the `apply-config` protocol scoot speaks is
  always the one the installed scootbg understands. If a user pins only
  one of them, `apply-config`'s protocol version check reports the
  mismatch in scoot's log. It must not fail silently.
- **Package default:** the flake wrapper injects
  `self.packages.${pkgs.system}.scootbg` with `mkDefault`, the same
  pattern as `programs.scoot.package`. A direct-module user (no flake)
  who turns it on without setting `package` gets a loud eval failure,
  not a session that silently has no wallpaper.
- **No per-user config from the NixOS side.** The config file stays
  per-user, in the home-manager module, as it does for scoot, and the
  NixOS module only guarantees the binary. `docs/nix.md` says so and
  shows the pairing next to the existing "complete session" example:
  home-manager `settings.wallpaper` plus NixOS `wallpaper.enable`.
- **scootbg is Linux-only** (`flake.nix` exposes `packages.*.scootbg`
  only where `isLinux`, and its `meta.platforms` is Linux). On Darwin the
  home-manager wrapper already defaults `programs.scoot.package` to
  `null`. `wallpaper.package` does the same there, and so does the
  overlay: it provides `pkgs.scootbg` only on Linux. A Darwin config
  that edits a `[wallpaper]` section for a Linux box must evaluate,
  not fail on a missing attribute.
- **An overlay:** `overlays.default` providing `pkgs.scoot`,
  `pkgs.scootctl` and `pkgs.scootbg`.
  - `docs/nix.md` currently says "there is no overlay", and that is why
    direct-module users must set `package` by hand.
  - Decide it in this item, and record the reason either way.
  - If it is added, the modules default to `pkgs.scootbg` where it
    exists, the flake wrappers keep injecting their own build, and the
    `docs/nix.md` caveat goes.
- **Checks:** extend `nix/tests.nix`, the hermetic `evalModules` checks
  that `nix flake check` runs. Cover the option off (no package), the
  option on (the package in `environment.systemPackages`), the
  direct-module failure without a package, the flake wrapper's default,
  and the overlay if it is added.
- **An end-to-end VM test:** a `nixosTest` that boots a scoot session
  and sees a wallpaper pixel would prove the whole path. Add it if CI
  can carry its cost; decide, and record which.
- **Docs, in the same PR:** `docs/nix.md`'s NixOS module table and prose
  (the new options, the pairing, the overlay), and the packaging line in
  `docs/scootbg/README.md`.

## Docs

`docs/configuration.md` gets a `[wallpaper]` section (with `command`), the
root `README.md` a line under "Running", and `docs/nix.md` the module
behaviour, all in the PR that lands this.

The scoot half goes through scoot's own review bar (it is compositor
code), with the smoke test extended to check a wallpaper pixel on each
headless output.
