---
title: "IPC focus actions run a full `apply` even when nothing moves"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# IPC focus actions run a full `apply` even when nothing moves

Found 2026-09-16, reviewing PR #54
(`resolved/ext-workspace-clicked-layer-keyboard-done.md`), deliberately
left out of that PR.

## The asymmetry

PR #54 gave `ext-workspace-v1`'s workspace activation an already-active
fast path: when the requested workspace is already active, it spends the
click and runs only the keyboard half (`refresh_keyboard_focus`) instead
of `act` — because otherwise a client repeating `activate` + `commit` as
fast as it can write to its socket drives a full `apply` (an arrange, a
configure per window, a render) per message, and the old code even said so
in its own comment.

The IPC focus family has no such fast path. `ipc.rs`'s
`Request::Action` handler converts and calls `self.act(...)`
unconditionally, so `focus-window-id` (or `focus-workspace-index`) for the
already-focused target runs the full `apply` every time. Same socket-speed
hazard, same cost, no guard.

## Why `low`, not higher

Exploiting it takes a client deliberately hammering focus actions at
socket speed — there is no evidence any real client does, and an agent
loop issuing one focus action per step pays one redundant arrange, which
is noise next to a screenshot encode. The ext-workspace guard exists
because that protocol *invites* repeat traffic (a bar re-asserting state);
nothing equivalent pressures the IPC path today. File now so the next
"focus feels slow under agent control" has somewhere to land; fix when a
measurement says so, not before.

## What it would take

Mirror the PR #54 split per focus variant: detect the no-op (target
already focused / already-active workspace) before `act` and run only the
keyboard half — with the `clicked_layer` clear PR #53 added kept on both
halves, and a test pinning each half (no-op spends the click without an
`apply`; real move still applies). Benchmark before/after on the IPC
dispatch path per the hot-path rule before claiming anything.
