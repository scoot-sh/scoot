---
title: "A screencopy frame parked for a session lock's blank is never re-armed once the vblank confirms"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# A screencopy frame parked for a session lock's blank is never re-armed once the vblank confirms

Found by a retrospective audit of PRs #73–#89 on 2026-09-17, traced against
`673c9ea`. Introduced by PR #84, which moved lock confirmation out of
`render()` and onto the DRM vblank under `--tty`.

`service_captures` parks a pending screencopy frame while a lock is waiting
for its blanked frame to reach scanout (`screencopy.rs:714`, gated on
`SessionLock::awaiting_blank`) — correct on its own: a capture must not
observe the pre-blank desktop.

The frame ticker, though, keeps running only for three conditions
(`headless.rs:914`):

```rust
if state.needs_render || !state.pending_idle.is_empty() || state.shots_draining() {
```

A parked screencopy frame is none of them, so the tick that rendered the
blank falls through to `state.timer_armed = false` (`headless.rs:917`) and
drops the timer. Confirmation then arrives from the DRM event handler via
`note_flip_completed` (`session_lock.rs:1317`), or from the fallback timer
via `note_blank_timeout` (`session_lock.rs:1333`) — and neither re-arms the
ticker. `ensure_ticking` does not appear in `session_lock.rs` at all, and
the DRM handler's own re-render is gated on `present_skipped`, which is
false in the normal case.

Net effect: once `awaiting_blank()` clears, nothing runs `service_captures`
again, so the parked frame is neither delivered nor failed. Before #84,
`confirm_lock` ran *inside* `render()`, so `service_captures` in that same
tick already saw `awaiting_blank() == false` and delivered.

Severity is bounded by what else re-arms the ticker: the locker's first
surface commit does, so in the ordinary case the frame is late by
milliseconds rather than lost. The indefinite case is a lock client that
takes the lock and never commits a surface — the abandoned-lock path the
lock code otherwise handles deliberately. `msg wait-idle` and `msg
screenshot` are unaffected, since `pending_idle` and `shots_draining()` are
both in the re-arm set above.

Fix shape: one `ensure_ticking()` on the confirm path, so the tick that
clears `awaiting_blank` is followed by one more. Worth deciding instead
whether a parked capture belongs in `frame_tick`'s own condition set
alongside `shots_draining()` — that is the version that cannot be forgotten
again by the next thing that moves work out of the render tick, which is
exactly how this arrived.

Testing note: this is invisible to the existing suite because both the park
and the confirm are exercised, just never in the same test — a regression
test wants a pending capture *and* a lock confirming on a vblank, asserting
the frame is delivered without any further client commit.
