---
title: "`ext_workspace.rs`'s cross-client check takes two backend locks per manager, per `wl_output` bind"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `ext_workspace.rs`'s cross-client check takes two backend locks per manager, per `wl_output` bind

Found 2026-09-16, reviewing PR #50, which had the same pattern and now uses
the cheaper form. Pre-existing in `ext_workspace.rs`, untouched by that PR.

## What it is

`workspace_group_output_bound` runs once per `wl_output` bind by *any*
client, and filters each registered manager so an `output_enter` never
carries another client's object — which is required: wayland-backend
*panics* on that ("Attempting to send an event with objects from wrong
client", `rs/server_impl/client.rs`). The filter is written as

```rust
if manager.manager.client().as_ref() != Some(&client) {
```

`Resource::client` (wayland-server 0.31.14, `src/lib.rs`) upgrades the
backend handle, calls `handle.get_client(self.id())` — which takes the
backend's state mutex — and then `Client::from_id`, which takes it again and
clones an `Arc<dyn ClientData>`. Two locks and an atomic refcount bump, per
manager, per bind, to answer a question that is a field comparison.

## The cheaper form, which is also the *exact* question

`ObjectId::same_client_as` (wayland-backend 0.3.17, `src/server_api.rs:173`)
compares the two ids' stored client ids and takes no lock at all:

```rust
if !manager.manager.id().same_client_as(&wl_output.id()) {
```

It is not merely cheaper, it is the same predicate the panic tests:
`o.id.client_id != self.id` on the object argument. A handle that has since
*died* is skipped under the system backend (`same_client_as` returns `false`
for a dead object, documented) and harmlessly kept under the Rust one, where
the send is swallowed as `InvalidId` rather than panicking — so the
substitution cannot weaken the safety property in either direction.

## Scope

Small, and worth doing with a benchmark rather than on faith — it is not a
per-frame path, and a session has a handful of managers, so this is tidying
a known cost rather than fixing a measured problem. `foreign_toplevel_
management.rs`'s `wlr_toplevel_output_bound` is the worked example to copy,
comment included. Its own per-bind walk is the larger of the two
(`binds × windows` against this one's `binds × managers`) and is already
filed for the object-count half in
`ext-workspace-object-binding-cap.md`.
