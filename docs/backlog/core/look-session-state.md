---
title: "Per-session look state: headless and --private-look sessions never touch the user's choice"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Per-session look state: headless and --private-look sessions never touch the user's choice

Filed 2026-10-12, split out of [`look-layer-scoot`](look-layer-scoot.md) for
the [`live-looks`](live-looks.md) epic. Serves **computer use**: an agent's
headless test session must be able to swap looks without repointing the
user's real desktop (D10).

## The gap

The look state is per user (`$XDG_STATE_HOME/scoot`) while sessions are per
display (the bar's socket is `scootbar-$WAYLAND_DISPLAY.sock`,
`crates/scootbar/src/control/paths.rs:1-8`). Without a scope, a swap inside a
headless test would show up on the user's desktop at its next reload.

## What to do

Per the design doc's section 4.2:

- `--tty` and `--nested` keep the user state: `--nested` is the webtop
  deployment (`site/src/content/docs/agents/webtop.md:17`) and the "try it
  first" path, and their swaps must survive a restart.
- `--headless` uses a private state under
  `$XDG_RUNTIME_DIR/scoot-look-<display>/`, seeded at startup from the user's
  `look.json`; any backend opts in with `--private-look`.
- Only a private session exports `SCOOT_LOOK_STATE`, to its direct
  children (beside `XCURSOR_*`, `state.rs:1850-1851`). The default path is
  deterministic, so nothing is added to `resources/scoot-session`'s
  environment imports, and systemd user units always act on the user state.
- `scoot look` resolves `<state>` as `--state`, `$SCOOT_LOOK_STATE`,
  `$XDG_STATE_HOME/scoot`, `~/.local/state/scoot`.
- Docs: the flag on the CLI pages; the private-state rule on the looks page.

## Evidence to record

Tests: a headless session's swap leaving the user state byte-identical; a
nested session's swap persisting across a restart; `--private-look` on
nested isolating it; the seed copied at startup.

## Not in this ticket

The IPC requests (`look-ipc`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
