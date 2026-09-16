---
title: "Screen capture: unbounded sessions per client, and unbounded frames per session."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Screen capture: unbounded sessions per client, and unbounded frames per session.

Found while building the output half of
[screencopy](../resolved/screencopy-capture-done.md) and filed rather than
fixed there, the same way the `wl_shm` per-pool cap names the total it does
not bound
([`shm-total-per-client-unbounded.md`](../security/shm-total-per-client-unbounded.md)).

Update 2026-09-16 (review): the cost model below was originally stated as
"N sessions cost N copies for the client that opened them" — true, but it
undersold two things a closer read of the pinned Smithay source
(`wayland::image_copy_capture`) found, and missed a second, independent
unbounded dimension entirely.

## Two unbounded dimensions, not one

**Sessions per client**, as originally filed: `screencopy.rs` throttles a
capture *session* well — a frame is parked rather than served on request,
the framebuffer read-back happens once per tick however many sessions want
it, and a session's second and later frames wait for the screen to actually
change. What is not bounded is how many sessions exist. N sessions each
with a parked frame cost N full-screen copies into N client buffers on every
tick the screen changes — ~8 MB each at 1920x1080.

**And the cost is not confined to the client that opened them.**
`ext_image_copy_capture_frame_v1.capture`'s dispatch (pinned rev,
`image_copy_capture/mod.rs:1321-1348`) linearly walks
`ImageCopyCaptureState::sessions` — the **global** list across every
client, not a per-client one — locking each session's mutex and scanning
its `active_frames` along the way; `FrameData::destroyed` (`mod.rs:1370-1385`)
walks the same global list on every frame teardown. So client A's session
count adds latency to client B's every `capture` request and every frame
destruction, not just cost to A's own captures.

**Frames per session, separately: also unbounded, and cheaper to exploit.**
`create_frame`'s handler (`mod.rs:1110-1123`) pushes onto a session's
`active_frames` with no cap and no pruning until the frame object is
destroyed — Smithay never posts the protocol's own `duplicate_frame` error
(see the resolved entry's "Bugs found and fixed" section). flexwm's
`Capture::pending: Option<Frame>` field does **not** bound this: `frame()`,
where that throttle lives, is only reached from the client's `capture`
request, never from `create_frame` itself. So a single session plus a
`create_frame` loop (no `capture` ever sent) is a cheaper version of the
same shape of attack — one client, one session, unbounded objects.

## Why it was not simply capped

- It is the same shape of per-frame, per-object work a client can already
  demand by mapping N surfaces, which this compositor accepts unbounded. A cap
  on one and not the other is inconsistent rather than safer.
- For the **sessions** dimension specifically: the only non-punitive form is a
  cap per *client*, and Smithay's `ImageCopyCaptureHandler` does not say which
  client a session belongs to: `capture_constraints` is handed an
  `ImageCaptureSource`, not a `Client`, and `new_session` a `Session`. A
  **global** cap would let one greedy client deny a well-behaved one, which is
  the concern already filed against the IPC connection cap
  ([`connection-cap-denies-the-same-user.md`](../ipc/connection-cap-denies-the-same-user.md)).

## What a fix would need

**Frames per session has a real hook today, unlike sessions per client.**
`dispatch.rs` already intercepts specific requests pre-delegation for exactly
this reason — `reject_oversized_shm_pool_creation` and three siblings — so a
`create_frame` cap (refuse a second `create_frame` before Smithay's handler
ever sees it, the same shape `duplicate_frame` itself would have been) is
achievable without needing per-client accounting at all: it only needs to
know whether *this session* already has an outstanding, unclaimed frame
object, which is local state this module already half-tracks.

**Sessions per client still needs a client-lookup path Smithay doesn't
expose today.** Either a per-client count (finding the client behind a
session via the session object's own `Resource::id()` through
`DisplayHandle::get_client`, reachable from `new_session` if Smithay exposes
the protocol object, or via a `new_with_filter` bind-time hook that counts
binds per client), or an argument that the per-surface precedent above makes
a cap the wrong tool and this half of the entry should close as won't-fix.

Whichever per-client accounting eventually lands for the three sibling
entries (`ext-workspace-object-binding-cap.md`,
`ext-workspace-client-lookup-per-bind.md`, and their own cross-references)
should cover the sessions half of this one too rather than being a fifth
independent mechanism — but the frames-per-session half is this protocol's
own shape and needs its own fix regardless.

Rough size: S for the frames cap (a `dispatch.rs`-style intercept); folds
into whatever closes the sibling per-client-accounting entries for the
sessions cap.
