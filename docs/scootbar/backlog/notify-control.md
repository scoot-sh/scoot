---
title: "scootnotify: `msg`, JSON, subscribe and wait, DND, history, rules, fullscreen hold"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-popups"
milestone: "M7"
---

# scootnotify: control, DND, history, rules

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#the-control-interface-and-what-an-agent-gets).
Serves **computer use** (a stable JSON view and `wait`) and **daily-drive** (DND, history,
rules).

## What to do

The control socket through `scootui` `control`: `list`, `history`, `dismiss`, `invoke`,
`restore`, `dnd`, `reload`, `subscribe`, `wait`, `version`, with the versioned JSON
object. DND mode, the history ring (in memory only), `[[rule]]` matching and actions
(`exec` with the bar's exec bounds), `fullscreen = "hold"`.

## Acceptance

Every verb tested end to end; `reload` answers applied or refused, as the live-looks contract's §4.2 requires; `wait` returns on a match and times out; a reload with a
bad file keeps the running config and state; subscriber and connection caps; a rule `exec`
flood bounded at 4 children; the JSON schema documented on the site in this PR.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The bar feed, Nix.
