---
title: "A client can make scoot hold ~900 fds that no per-client cap counts (syncobj points on destroyed timelines; dmabuf params adds)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Client-held fds no cap counts

Filed 2026-09-24 from the review of PR #233 (explicit sync). It serves
**daily-drive**: a client-triggerable resource exhaustion that sheds other
clients.

## What is wrong

`fd_pressure.rs` is sized on the claim that every fd a client can make scoot
hold is counted by some per-client cap (buffers, pools, timelines, waits),
so one connection cannot exhaust the table alone and the pressure guard's
kill lands on a contributor. Review found two paths that break it.

1. **Syncobj points on destroyed timelines** (GPU scanout tier, where
   explicit sync is offered). An imported timeline keeps the client's
   syncobj fd open in scoot, and every `DrmSyncPoint` on it holds the
   timeline's `Arc`. A point set on a surface outlives the destruction of
   the timeline object (the protocol requires that), but
   `drm_syncobj::forget_destroyed` releases the 128-timeline count on
   destroy. So "import, set points on a fresh surface, destroy the
   timeline", repeated, keeps one fd per surface with zero live timelines
   counted.
2. **dmabuf `params` adds that are never created** (every tier with the
   dmabuf global). Each `zwp_linux_buffer_params_v1.add` hands scoot a
   plane fd, which the params object keeps until it is used or destroyed.
   A client that creates params objects, adds planes and never calls
   `create`/`create_immed` keeps the fds, and no cap counts params
   objects or their planes (`wl_buffers.rs` counts only buffers that
   were created).

## Measured (reviewer, dev VM, 1024-fd table)

- Path 1: 440 surfaces with pending points on destroyed timelines put
  **927 fds** in scoot with **0 live timelines**.
- Path 2: 220 params objects with 4 adds each, never created, put the same
  927 fds in scoot.
- In both, new clients were shed at accept (the pressure reserve), and
  **the offender was not killed**: none of its creations is past a counted
  grace, so fd pressure's creation guard cannot pick it. The kill is meant
  to land on a contributor; here the contributor holds uncounted fds.

## What to do (not done in PR #233)

Count what actually retains fds, per client:

- params planes: claim on `add`, release on params destroy or consume, cap
  and grace like buffers;
- retained syncobj timelines: claim on import and release when the
  `DrmTimeline` is really freed, not when its object is destroyed. That
  needs a drop hook scoot can see, for example counting fds through a
  wrapper, or another Drop-side change in the Smithay fork
  ([repin ticket](./smithay-fork-repin.md));
- alternatively, make fd pressure attribute fds per client (e.g. from the
  objects each client holds) so the kill can find the real holder.

Whichever it is, re-measure both paths live, and fix `fd_pressure.rs`'s
module doc, which now states the gap.
