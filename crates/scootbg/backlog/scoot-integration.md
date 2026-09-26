---
title: "Seamless in scoot: a [wallpaper] config section"
status: "open"
area: "scootbg"
priority: "high"
blocked: "needs scootbg set, query and the daemon to exist first"
---

# Seamless in scoot: a [wallpaper] config section

In scoot, the wallpaper should be one config section, with nothing else to
wire up.

```toml
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```

- **scoot owns startup.** With a `[wallpaper]` section, scoot spawns
  `scootbg daemon` itself (before `[autostart]`, so the wallpaper is up
  before the bar), then `scootbg set` per output. No `[autostart]` entry
  needed; one there as well must not start a second daemon (the daemon's
  own "already running" refusal covers it).
- **One-way coupling.** scoot depends on scootbg's CLI and its exit codes,
  never its crate. scootbg never reads scoot's config. The scoot side
  lives in the compositor (config parsing, spawn, reload), not in
  `scoot-core`, which stays platform-independent.
- **Reload.** `scootctl reload` re-applies `[wallpaper]` only if the
  section changed since the last apply, so a wallpaper picked with
  `scootbg set` survives unrelated config edits. Removing the section runs
  `scootbg clear`; it does not kill the daemon.
- **Failure is loud but harmless.** A missing `scootbg` binary, a bad
  path or a decode error logs a warning and leaves `background_color`
  showing. The session always starts (the same fail-open rule as
  `[autostart]`).
- **No flash of the wrong colour.** Until scootbg commits its first
  buffer, scoot shows `background_color`. Measure the gap on `--tty`; if
  it is visible, scootbg sets a colour first (single-pixel, instant) and
  the image after.
- **Packaging.** The scoot Nix package ships `scootbg` alongside `scoot`
  and `scootctl`, so the config section works with no extra install; the
  home-manager module renders `[wallpaper]` from `settings` like any other
  section.
- **Docs.** `docs/configuration.md` gets a `[wallpaper]` section, and the
  root `README.md` a line under "Running".

The scoot half goes through scoot's own review bar (it is compositor
code), with the smoke test extended to check a wallpaper pixel on each
headless output.
