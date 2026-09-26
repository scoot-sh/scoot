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

Mechanism: scoot starts `scootbg daemon --initial <values from the
section>`. The daemon keeps, in its state file, a fingerprint of the last
`--initial` it applied. At startup, if the fingerprint matches, the section
has not changed since, so it restores its own state (which includes any
later `scootbg set`); if it differs, it applies the section and records the
new fingerprint. Passing the values as daemon arguments also removes any
race between spawning the daemon and its socket being ready.

## How scoot drives it

- **Startup.** With a `[wallpaper]` section, scoot spawns `scootbg daemon
  --initial ...` itself, alongside `[autostart]`. No autostart entry is
  needed, and one there as well does not start a second daemon (the
  daemon's own "already running" refusal). Spawn order does not decide
  which client commits first, so nothing here promises the wallpaper
  appears before a bar.
- **Reload.** `scootctl reload` re-applies `[wallpaper]` only if the
  section changed since the last apply, by spawning `scootbg set ...`.
  Removing the section spawns `scootbg clear`; it does not kill the daemon.
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
  not to paint a different placeholder colour, which would be its own
  flash.

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

## Docs

`docs/configuration.md` gets a `[wallpaper]` section (with `command`), the
root `README.md` a line under "Running", and `docs/nix.md` the module
behaviour, all in the PR that lands this.

The scoot half goes through scoot's own review bar (it is compositor
code), with the smoke test extended to check a wallpaper pixel on each
headless output.
