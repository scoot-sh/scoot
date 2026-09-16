---
title: "Screen capture: how many sessions one client may hold is unbounded."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Screen capture: how many sessions one client may hold is unbounded.

Found while building the output half of
[screencopy](../resolved/screencopy-capture-done.md) and filed rather than
fixed there, the same way the `wl_shm` per-pool cap names the total it does
not bound
([`shm-total-per-client-unbounded.md`](../security/shm-total-per-client-unbounded.md)).

`screencopy.rs` throttles a capture *session* well: a frame is parked rather
than served on request, at most one frame may be outstanding per session, the
framebuffer read-back happens once per tick however many sessions want it,
and a session's second and later frames wait for the screen to actually
change. What is not bounded is **how many sessions exist**. N sessions each
with a parked frame cost N full-screen copies into N client buffers on every
tick the screen changes — ~8 MB each at 1920x1080 — so a client that opens
many sessions can multiply the compositor's per-frame memory traffic by N.

## Why it was not simply capped

- It is the same shape of per-frame, per-object work a client can already
  demand by mapping N surfaces, which this compositor accepts unbounded. A cap
  on one and not the other is inconsistent rather than safer.
- The only non-punitive form is a cap per *client*, and Smithay's
  `ImageCopyCaptureHandler` does not say which client a session belongs to:
  `capture_constraints` is handed an `ImageCaptureSource`, not a `Client`, and
  `new_session` a `Session`. A **global** cap would let one greedy client deny
  a well-behaved one, which is the concern already filed against the IPC
  connection cap
  ([`connection-cap-denies-the-same-user.md`](../ipc/connection-cap-denies-the-same-user.md)).

## What a fix would need

Either a per-client count, which means finding the client behind a session
(the session object's own `Resource::id()` resolves to one through
`DisplayHandle::get_client`, reachable from `new_session` via
`SessionRef`'s protocol object if Smithay exposes it, or via a
`new_with_filter` bind-time hook that counts binds per client), or an
argument that the per-surface precedent above makes a cap the wrong tool and
this entry should be closed as won't-fix. Either is a real answer; what this
entry exists to prevent is the question quietly going unasked.

Rough size: S.
