---
title: "`flexwm msg outputs` reports only an output's full rectangle"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# `flexwm msg outputs` reports only an output's full rectangle

`flexwm msg outputs` reports only an output's full rectangle, so an
agent cannot see what a bar reserved (item 14 gave the core a `usable`
area but did not extend the IPC surface). Adding a `usable` rect to
`OutputSnapshot` is a one-field change to `flexwm-ipc`. It does **not**
need a `PROTOCOL_VERSION` bump: output scaling added the `scale` field the
same way — `#[serde(default)]`, so an old server's reply still decodes and
the tagged `Response` discriminant is unchanged. This entry previously
assumed a bump; the `scale` field disproved that premise. It is still
worth landing alongside an IPC action for "focus workspace N", which
item 15 deferred for the same reason: `flexwm_core::Action::FocusWorkspaceIndex`
already exists (it is what `ext-workspace-v1`'s `activate` drives), so all
that is missing is the wire half — a variant on `flexwm-ipc`'s own
`Action` mirror carrying the index, its `convert.rs` arm, its
`msg action` spelling in `cli.rs`, and its `README.md` row. Until then an
agent can only step workspaces one at a time (`focus-workspace up|down`)
while a bar speaking `ext-workspace-v1` can jump straight to one. Landing
them together means one review of the wire surface, not one forced bump.
