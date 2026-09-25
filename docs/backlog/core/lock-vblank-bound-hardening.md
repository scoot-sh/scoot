---
title: "Lock-confirm bound wording, and aging out stale dumb-tier vblanks"
status: "open"
area: "core"
priority: "low"
blocked: "none"
---

# Lock-confirm bound wording, and aging out stale dumb-tier vblanks

Filed 2026-09-25 from the round-2 review of PR #247 (milestone 19 phase E).
Neither item blocked that merge. Both are hardening: one is a doc
inaccuracy, the other a failure that needs a driver fault to reach.

## 1. `await_vblank`'s "late, never early" wording

`session_lock.rs` (the `await_vblank` doc) says the fallback bound is "late,
never early, for a frame that has drawn". Since #247 the deadline is armed
once per lock wait and never extended. That makes it true only in the
literal sense that the timeout never fires before a blank has drawn.

Compared with `main` before #247, there is one narrow window where it now
fires earlier. A discard plus re-present in the last flip-latency before the
first bound lets the fixed deadline fire before the re-presented blank's own
flip completes.

**Fix:** reword to what the code guarantees: "no later than one bound after
the first blank drew, and never before a blank has drawn". No behaviour
change.

## 2. A never-delivered owed vblank can freeze a reused CRTC (dumb tier)

`Tty::stale_vblanks` records the CRTC of a head removed by hotplug while its
dumb-tier flip was in flight. The next `VBlank` on that CRTC is eaten, so
it can't settle a new head built on the same CRTC.

In normal operation the owed event is always queued first. The surface's
`Drop` does a blocking `ALLOW_MODESET` commit (`surface/atomic.rs:985` at
the pinned Smithay rev), and the kernel holds that until the earlier flip
completes.

**The residual:** if a driver never delivers that event (a kernel commit
stall timeout, or a driver fault), the entry stays until the CRTC is reused.
It then eats the new head's first real vblank, and that screen freezes until
a VT switch or a DRM event error clears the list.

**Fix:** timestamp each entry when it is pushed, and ignore (drop) any
entry older than about one second when a vblank arrives. The owed event is
already queued at push time, so a genuine stale event always arrives well
inside that window.

**Pin:** push a stale entry, advance a synthetic clock past the age, then
deliver a vblank for a new head on that CRTC. The new head must settle.

The GPU tier has no equivalent guard, because `DrmCompositor`'s queue is
private. Its residual (a stale completion settles a new frame at most one
vblank early) is documented in `scanout.rs`.
