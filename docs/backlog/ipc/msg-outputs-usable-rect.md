---
title: "`flexwm msg outputs` reports only an output's full rectangle"
status: "open"
area: "ipc"
priority: "medium"
blocked: "bundle with PROTOCOL_VERSION bump"
---

# `flexwm msg outputs` reports only an output's full rectangle

`flexwm msg outputs` reports only an output's full rectangle, so an
agent cannot see what a bar reserved (item 14 gave the core a `usable`
area but did not extend the IPC surface). Adding a `usable` rect to
`OutputSnapshot` is a one-field, version-bumping change to `flexwm-ipc`;
it is worth doing alongside whatever else next changes that wire format
rather than bumping `PROTOCOL_VERSION` on its own. Bundle it with an
IPC action for "focus workspace N", which item 15 deferred for exactly
the same reason: `flexwm_core::Action::FocusWorkspaceIndex` already exists
(it is what `ext-workspace-v1`'s `activate` drives), so all that is
missing is the wire half — a variant on `flexwm-ipc`'s own `Action` mirror
carrying the index, its `convert.rs` arm, its `msg action` spelling in
`cli.rs`, and its `README.md` row. Until then an agent can only step
workspaces one at a time (`focus-workspace up|down`) while a bar speaking
`ext-workspace-v1` can jump straight to one. Whoever bumps
`PROTOCOL_VERSION` for either should land both.
