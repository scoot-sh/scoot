---
title: "Config file, control socket and reload: `scootbar msg`, `query` for agents"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock"
milestone: "M3"
---

# Config file, control socket and reload

Filed 2026-09-29. Serves **daily-drive** (configurable) and **computer use**
(`query` reads the bar as data instead of OCR).

## Config

Its own file (`$XDG_CONFIG_HOME/scoot/bar.toml`, name to settle), separate
from scoot's `config.toml`, so it works on other compositors and a bar change
never breaks scoot. Sections: bar (edge, height, margin, radius, opacity,
font file and size), `left`/`center`/`right`, color tokens, per-module
tables. Unknown keys are a loud error naming the key; a bad reload keeps the
running config (as scoot's does). Parser per the
[spikes](baselines-and-spikes.md).

## The parser spike, first

`toml` (used elsewhere in the tree) vs a smaller one (`basic-toml`) vs a
hand-rolled subset: size and parse time on the real schema, and how each fails on
malformed input (a clear error naming the key, never a panic). Moved here from
[baselines-and-spikes](baselines-and-spikes.md); the first milestones ran on
command-line flags and did not need it.

## Control socket and CLI

A lock-guarded, `0600`, same-user socket with line-framed JSON, like scootbg's
(`crates/scootbg/src/framing.rs`, `protocol.rs`). `scootbar msg`:

- `set ID JSON` (see [exec-push-button-modules](exec-push-button-modules.md))
- `query` returns each module's current state as JSON (text, class, value
  where it has one): battery, volume, network, workspace. This is the agent
  hook.
- `reload`, `version`, `kill`, as scootbg has.

Reload also on SIGHUP. Launch is from scoot's `[autostart]` today; a `[bar]`
convenience like `[wallpaper]` is a later choice, not this ticket.

## Docs

`docs/scootbar/README.md` and `docs/scootbar/cli.md` in the same PR as the
first config key and command, per `CLAUDE.md`: what and why in the README, keys
and flags in the reference.

## Done when

Editing the file and running `scootbar msg reload` changes a live bar,
`query` returns valid JSON for every module, and malformed config is refused
without disturbing the running bar.
