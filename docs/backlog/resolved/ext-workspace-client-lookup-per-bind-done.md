---
title: "`ext_workspace.rs`'s cross-client check took two backend locks per manager, per `wl_output` bind — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `ext_workspace.rs`'s cross-client check took two backend locks per manager, per `wl_output` bind — RESOLVED.

## The entry as filed

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

## Resolution (2026-09-17, PR #68)

Fixed exactly as filed: one predicate swap plus the comment, copied from
`wlr_toplevel_output_bound` (PR #50) statement for statement, manager for
handle. No other instance of the pattern exists (see the audit below), so
the PR is this plus one test. No README change: no user-facing behavior
changes in any direction — same events, same order, same clients.

### The fix

`workspace_group_output_bound` (`ext_workspace.rs`) now holds
`let bound = wl_output.id();` and filters with
`!manager.manager.id().same_client_as(&bound)`. The old form's
`wl_output.client()` once-per-bind lookup goes away with it — there is no
`None` arm left to keep, a freshly bound output's id always names its
client.

### The test

`an_output_bound_by_one_client_never_enters_another_clients_group`
(`ext_workspace/tests/mod.rs`): two real clients, the second binding its
manager first and its `wl_output` after so the hook fires while the first
client's manager is registered. It asserts the first client saw nothing
and the second saw exactly its own world (bind-time burst, then
`OutputEnter` + `done`). With the filter neutered to `if false` the test
does not fail an assertion — the compositor panics:

```
thread '...an_output_bound_by_one_client_never_enters_another_clients_group'
panicked at .../wayland-backend-0.3.17/src/rs/server_impl/client.rs:194:29:
Attempting to send an event with objects from wrong client.
```

With the fix restored: `36 passed; 0 failed`. The pre-existing
cross-client tests (`two_clients_are_kept_in_step_independently`,
`one_client_disconnecting_does_not_disturb_another`) also pass unchanged,
which is the other half of "a test that would have panicked before still
cannot panic after".

### The dead-manager edge is not constructible — stated, not tested

A dead manager sitting in `managers` at `output_bound` time cannot be
built through real client behavior: `stop` removes the entry before
`finished`, client-side destroy and disconnect both run the same
`retain` in `destroyed`, the refresh path drops dead clients in
`retain_mut`, and everything runs on the single event-loop thread, so no
bind can interleave between a death and its prune. The old `client() →
None → skip` arm and the new backend-dependent dead arm were both
unreachable defense-in-depth. Under the live Rust backend a hypothetically
kept dead manager still could not panic: its group `Weak` would fail to
upgrade first, and even past that the send dies as `InvalidId` (see
below), never at the client-id panic check.

### Upstream claims, re-derived against the pinned revs (not assumed)

- `Resource::client` (wayland-server 0.31.14, `src/lib.rs:152-157`):
  `handle.get_client(self.id())` (one state-mutex lock,
  wayland-backend 0.3.17 `rs/server_impl/handle.rs:133-135`) then
  `Client::from_id` → `get_client_data` (second lock, same file
  `:137-139`) cloning an `Arc<dyn ClientData>`. Two locks and a refcount
  bump per call, as filed.
- `same_client_as` (wayland-backend 0.3.17, `src/server_api.rs:173`):
  delegates to `InnerObjectId::same_client_as`. Rust impl
  (`rs/server_impl/mod.rs:36-38`) is `self.client_id == other.client_id`
  — a field comparison, no lock. Line number exact.
- The panic is literally `o.id.client_id != self.id`
  (`rs/server_impl/client.rs:174-175` for `NewId`, `:193-195` for
  `Object`), so the new predicate is the exact question, as filed.
- `InvalidId` swallow: the `?` on `get_object` (same file `:177`, `:196`)
  propagates out of `send_event` into the generated event method, which
  discards it — wayland-scanner 0.31.11 `src/server_gen.rs:267`:
  `let _ = self.send_event(...)`. Confirmed in the generator, not inferred
  from the call site.
- Backend: flexwm builds wayland-backend 0.3.17 with **no features**
  (`cargo tree -e features -p wayland-backend` on the dev VM prints an
  empty feature set; nothing in the repo enables `use_system_lib`,
  `system`, or `server_system`), and `lib.rs:89-90` selects
  `server = rs::server` without `server_system`. The system-backend half
  of the ticket's dead-object analysis is therefore moot for this build —
  the comment covers both anyway, like PR #50's. The fail-first panic
  above landing at `rs/server_impl/client.rs:194` is live confirmation
  the Rust backend is the one running.

### Benchmark: the honest noise verdict

Predicate micro over live objects (temporary harness test, 200k
iterations, dev VM debug build, `--test-threads=1`, removed before
commit), three runs:

```
client()=330ns/iter  same_client_as=34ns/iter
client()=335ns/iter  same_client_as=30ns/iter
client()=350ns/iter  same_client_as=31ns/iter
```

~10x on the predicate, ~300ns absolute per manager per bind. A session
holds a handful of managers, so a bind saves on the order of a
microsecond against a bind's real cost (socket round trips, registry
announcement, the output walk itself) — noise, exactly as the ticket's
scope note anticipated. Merged on correctness-clarity grounds (the cheaper
form is also the exact predicate), not on a performance claim. No
end-to-end bind-storm run: at socket-I/O-dominated per-bind costs the
~300ns delta would be invisible by construction, so the predicate
decomposition above is the measurement, not a proxy for one.

### Elsewhere audit: no other instance

`Resource::client()` / `get_client` across `crates/`:

- `foreign_toplevel_management.rs:321` (`open_wlr_toplevel`): needs the
  `Client` object itself to `create_resource` new handles; once per
  window-open per manager, not per bind. Not the pattern.
- `ext_workspace.rs:217` (`Manager::apply`), `output_management.rs:326`:
  `DisplayHandle::get_client` — a single-lock lookup, and the `Client` is
  needed to create objects. Not the pattern.
- `handlers.rs:381` (`focus_changed`), `state.rs:755` (`client_of`),
  `dispatch.rs:605`: same single-lock lookup where the owner has to be
  *discovered* (selection focus, per-event serial ownership, icon
  recovery). `same_client_as` cannot answer those — it needs a second id
  to compare against. Not the pattern.
- `handlers.rs:556,559` (`dnd_source_client`): `Resource::client()` but
  once per drag-start to learn the owner for serial validation. Not a hot
  filter. Left alone.

### Evidence

All captured on the dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`),
against commit `1c73c4b` (the fix; bookkeeping followed as a second
commit on the same branch):

```
cargo test -p flexwm            TEST EXIT=0  727 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  826 run: 826 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0 (15 `ok:` lines)
```

Baseline before the change was 726 passed (the +1 is the new test).
Fail-first toggle done via Mac-side edits (no `git stash` from the VM
side per the 9p index-lock gotcha); the neutered-filter run above is the
record.
