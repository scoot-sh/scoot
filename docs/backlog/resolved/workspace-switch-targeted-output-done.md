---
title: "Switch a specific output's workspace, not only the focused output's"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Switch a specific output's workspace, not only the focused output's

Filed 2026-09-29 (scootbar planning). Serves **daily-drive** (a per-output
bar's workspace clicks) and **computer use** (an agent addressing a monitor
by id instead of stealing focus to it first).

## The gap

`focus-workspace-index N` acts on the focused output's list
(`compositor/ipc.rs`, `Action::FocusWorkspaceIndex`). The `ext-workspace-v1`
`activate` handler reaches the same action, so it refuses a switch on any
other output: `compositor/ext_workspace.rs` returns early, with a
`debug_assert!(false, "workspace switch for a non-focused output has no
output-targeted action yet")`, rather than switch the wrong screen. Today
that is unreachable, because the guard's own comment says every other
output's list is the single empty workspace while windows open only on the
first output. Once placement changes
([multi-output remainder](../core/multi-output-remainder.md)) it is
reachable, and the `debug_assert!` is a panic in debug builds and a silently
dead click in release.

A bar with one surface per output shows each output's own group
(`docs/protocols.md`), so clicking workspace 2 on the second monitor must
switch that monitor.

## What to do

An output-targeted core action, so both transports name the output:

- Core: `FocusWorkspaceIndex` gains an optional output (or a sibling
  `FocusOutputWorkspaceIndex { output, index }`). `scoot-core` stays
  platform-independent, so this is an action with an output id like
  `move-window-to-output ID`, nothing more.
- IPC: `focus-workspace-index N [--output ID]`; without it, unchanged
  (focused output). A protocol bump only if the wire shape needs one.
- `ext_workspace.rs`: `activate` on a non-focused output's workspace goes
  through the targeted action; the early return and the `debug_assert!` go.
  Decide, and pin in a test, whether the switch also moves focus to that
  output (a click on a bar is an interaction with that monitor, so
  probably yes, matching how a click on a window focuses its output).
- Keep the already-active fast path and the `clicked_layer` spend the
  handler documents; they must not differ by transport.

## Edge cases to pin

An output id that has since been removed; a stale index after the list
shrank; a locked session (already refused first); a switch targeting an
output whose workspaces were adopted by another (index refers to the
adopter's list, `docs/protocols.md`).

## Not in this ticket

Per-output workspace *creation* or `assign`; those stay ignored.

## Resolution (2026-10-04, PR #409)

Landed as `feat(scoot-core,scoot-ipc,scoot,scootctl): switch a specific output's workspace` (`e01b1daf`). Sibling action `FocusOutputWorkspaceIndex { output, index }` (not an optional field — serde would silently misroute to the focused output on old servers); focus-follows YES including already-active; no protocol bump (stays 7); `debug_assert!(false)` gone. Review verified the serde claim empirically + all read sites + edge cases; one low docs-clarity fix landed in follow-up. CI green.
