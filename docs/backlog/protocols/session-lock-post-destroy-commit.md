---
title: "Committing a lock surface after its role is destroyed kills the client."
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# Committing a lock surface after its role is destroyed kills the client.

The lock-surface half of the DMS post-unlock kill (`dms-enablement-gaps.md`,
gap 1). The layer-surface half -- overlay dismissal killing the whole shell
-- is fixed (neutralize the pending anchors once Smithay's destruction
handler has run; see `layer_shell.rs`'s `neutralize_destroyed_layers`).
This half is the same bug *family* in a different hook, and the layer fix
does not transfer. Pinned by
`session_lock/tests.rs::committing_a_lock_surface_after_its_role_was_destroyed_is_refused`
(which asserts the kill; inverting it is how the fix proves itself).

## Symptom

Lock, map a lock surface, destroy only its `ext_session_lock_surface_v1`
role object (keeping the `wl_surface`, the lock and the connection alive --
legal, and what a locker does when an output is removed under it), then
commit on the surviving surface. The server answers:

```
wl_display@1.error(ext_session_lock_surface_v1@18, 0,
    "Committed before the first ack_configure.")
```

i.e. `CommitBeforeFirstAck`, and the connection is dead. Reproduced
deterministically in-harness (no DMS needed); wire captured with
`WAYLAND_DEBUG=1` on a `cargo test -p flexwm --bin flexwm
session_lock::tests::committing_a_lock_surface` run.

## Root cause

Same shape as the layer kill, in the pinned Smithay rev (`0ff0098`):

- `wayland/session_lock/surface.rs`'s `destroyed()` resets the role
  attributes and the cached pending/current state to default -- including
  `last_acked: None`.
- The next `wl_surface.commit` runs that module's `pre_commit_hook`, whose
  *first* check requires `role.last_acked` and posts
  `CommitBeforeFirstAck` without it.

There is a second, independent facet: a *null* commit after role destroy
hits `NullBuffer` ("Surface attached a NULL buffer"), which is a **by-design**
error for mapped lock surfaces (the protocol forbids unmapping one) applied
to a surface that no longer has a role at all.

## Why the layer fix does not transfer

1. **No role-to-surface mapping survives the unlock.** The layer neutralize
   works because `layer_destroyed` hands flexwm the `wl_surface` before
   Smithay's reset runs. The session-lock destruction path offers no such
   callback -- and worse, the real unlock order (`unlock_and_destroy`, which
   clears `SessionLock::surfaces`, *then* role teardown) means that by the
   time the role is destroyed flexwm has deliberately forgotten every lock
   surface. Reaching the surface to neutralize it would mean keeping zombie
   mappings past `unlock`, against the takeover-safety logic that clears
   them.
2. **There is nothing honest to write.** The layer neutralize writes a legal
   anchor shape the client could have asked for. The lock hook demands a
   previously-acked configure; the only value that passes is one the client
   did ack but Smithay's reset dropped -- recoverable only by capturing it
   *before* destruction (a pre-destroy hook flexwm would have to add in
   `dispatch.rs`'s request path) and restoring it after, and even that
   leaves the `NullBuffer` facet unfixable through any public API.

Candidate resolutions, in increasing order of invasiveness: (a) confirm
what quickshell's real unlock teardown sends -- if it never commits after
role destroy, this entry's urgency drops to "hardening"; (b) an upstream
Smithay fix that skips role validation for commits on role-less surfaces
(the wlroots behavior: the role is gone, the commit is a no-op); (c) the
capture-and-restore sketched above for the bare-commit facet only.

## What lands with the fix

- Invert the pinned test (client survives, unlock still works).
- The new `disconnected` warn log (`state.rs`) already names the code,
  object and message for the next such kill -- that half needs no change.
