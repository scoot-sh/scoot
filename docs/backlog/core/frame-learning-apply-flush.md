---
title: "A learned column minimum from a client frame is not applied until the next unrelated event"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Frame learning never flushes its own relayout

Filed 2026-09-23 by the PR #223 re-review (pre-existing). `observe_frame`
feeds `FrameObserved` into the core (`handlers.rs:~144`), which can raise a
learned minimum and re-scroll, but nothing calls `apply()` after it, so the
new column width and positions reach clients and the screen only at the next
event. PR #223's no-op-fullscreen fast path removed one place that used to
flush this by accident.

Fix: apply (or schedule one coalesced apply per dispatch) when the core
reports that a frame changed the arrangement — never per commit
unconditionally, since commits arrive at frame rate. Pin with a harness test:
a refusing frame changes the placed rect without any further input.
