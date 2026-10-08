---
title: "Config file, control socket and reload: `scootbar msg`, `query` for agents"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-09-29"
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
running config (as scoot's does). The parser is chosen by the spike below.

## The parser spike, first

`toml` (used elsewhere in the tree) vs a smaller one (`basic-toml`) vs a
hand-rolled subset: size and parse time on the real schema, and how each fails on
malformed input (a clear error naming the key, never a panic). Moved here from
[baselines-and-spikes](resolved/baselines-and-spikes-done.md); the first milestones ran on
command-line flags and did not need it.

## Control socket and CLI

A lock-guarded, `0600`, same-user socket with line-framed JSON, like scootbg's
(`crates/scootbg/src/framing.rs`, `protocol.rs`). `scootbar msg`:

- `set ID JSON` (see [exec-push-button-modules](exec-push-button-modules-done.md))
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

## Resolution (2026-09-29, PR #339)

Shipped as `scootbar msg` (`query`, `reload`, `version`, `kill`, `set ID JSON`
validated but refused — no module takes sets yet, the hook for
`exec-push-button-modules`) plus `$XDG_CONFIG_HOME/scoot/bar.toml` (`--config`
override; defaults < file < flags, flags re-overlaid on every reload) and a
lock-guarded `0600` socket following scootbg's framing/claim design. Parser
spike (`dev/spikes/scootbar/config-parser.md`) chose the workspace's existing
`toml` (zero new `Cargo.lock` packages). `query` returns every placed module's
text/class/icon per output — the agent hook. Review caught two real bugs
before merge: reload not forcing a redraw (fixed via canvas clear), and the
empty-bar corner (`Output::invalidate`). Evidence: `nextest -p scootbar`
247/247, clippy/fmt clean, live `tests/msg.rs` pixel test against headless
scoot; binary +369 KB release. Deliberate deviations: SIGHUP keeps default
action (`forbid(unsafe_code)` + no signalfd in rustix; reload via
`msg reload`), no scoot `[bar]` section (out of scope), no `value` key in
query (room reserved).
