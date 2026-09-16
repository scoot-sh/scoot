---
title: "Screenshot capture and PNG encode run on the sole event-loop thread (MEDIUM)"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# Screenshot capture and PNG encode run on the sole event-loop thread (MEDIUM)

Split out of
[the connection-cap entry](../resolved/ipc-connection-cap-resolved.md) when
its other two concerns were closed (2026-09-16); this is the one that was
always going to be its own piece of work, and it is unaffected by both fixes
that landed there.

A `Request::Screenshot` costs a full render, a framebuffer read-back and a
PNG encode, all on the compositor's only thread — the one that also runs
wayland dispatch, input and every other IPC connection. Each capture stalls
all of that for its duration: **~12ms at 1600x1000 in a release build**,
measured in item 9.

What is already done and does *not* close this:

- **The per-connection rate limit** (item 9): one capture per connection per
  16ms frame, refused rather than delayed. It bounds how often the stall can
  happen, not how long one lasts.
- **The connection cap** (`ipc/slots.rs`): 64 connections, so the rate limit
  can no longer be multiplied by reconnecting. Same thing — a bound on
  frequency, not on the stall itself. Worth noting for whoever picks this up:
  64 connections each allowed a capture per frame is still, in the limit, far
  more capture work than one thread can do in a frame, so the cap makes the
  worst case finite rather than acceptable.

Moving the encode (and possibly the read-back) off-thread is a real
architectural decision, not a rider on a bound: a thread pool, an async
encode, or a dedicated worker each imply something different about how the
reply is delivered (the connection that asked has already left `serve` by
then), how a client disconnecting mid-encode is handled, and what happens to
`wait-idle`'s notion of a quiet screen while a capture is in flight. It
deserves its own design pass.
