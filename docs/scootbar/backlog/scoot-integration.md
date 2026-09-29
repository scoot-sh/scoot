---
title: "Seamless in scoot: a `[bar]` section that starts, reloads and restarts the bar"
status: "open"
area: "scootbar"
priority: "low"
blocked: "config-cli-and-reload"
milestone: "ongoing"
---

# Seamless in scoot

Filed 2026-09-29. Serves **daily-drive**. The bar ships standalone first
(launched from `[autostart]`, own config file); this is the optional polish,
the way [scootbg's `[wallpaper]`](../../scootbg/backlog/resolved/scoot-integration-done.md)
was.

## What it would add

- A `[bar]` section in scoot's `config.toml` that starts `scootbar` at
  session start and re-applies on `scootctl reload`, so no autostart line.
- **Restart on crash.** Scoot does not supervise clients today, and the bar's
  release profile aborts on a panic. Decide whether scoot restarts a bar that
  exits abnormally (bounded backoff, never a loop) or leaves that to a systemd
  unit ([robustness-and-limits](robustness-and-limits.md)).
- Scoot-side defaults (a bar height that matches its gap and ring settings).

## The real question

Coupling. The bar's config staying its own file is what lets it run on other
compositors and lets a bar change never break scoot. Any `[bar]` section must
be a thin launcher and pass-through (like `apply-config`), not a second schema
for the same keys. If the cost is a second place to look for options, the
standalone `[autostart]` line is the better default and this entry is a
deliberate refusal.

## Done when

Decided either way with the reasoning recorded; if built, `scootctl reload`
re-applies the bar and a crashed bar comes back at most a bounded number of times.
