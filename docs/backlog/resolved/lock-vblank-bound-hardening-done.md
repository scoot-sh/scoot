---
title: "Lock-confirm bound wording, and aging out stale dumb-tier vblanks — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Lock-confirm bound wording, and aging out stale dumb-tier vblanks — RESOLVED

RESOLVED 2026-09-28 (PR #303). Half 1: `await_vblank`'s doc now says what
the code guarantees since #247 -- the fallback confirms no later than one
bound after the first blank drew, and never before a blank has drawn (the
deadline is armed once per wait and never extended, verified at every site
that arms or uses it: `await_vblank`, `confirm_on_vblank`,
`poll_blank_timeout`, `cancel_blank_wait`, `note_blank_timeout`, and the
render-tail timer). The two `headless.rs` comments that still described the
pre-#247 restarted bound (the render-tail timer note and `blank_timeout`'s
doc) are corrected to match. No behaviour change. Half 2:
`Tty::stale_vblanks` is a timestamped `StaleVblanks` list
(`crates/scoot/src/compositor/tty/stale_vblanks.rs`): each entry is stamped
at push in `hotplug.rs`, and `on_vblank` (taking the vblank's arrival
`Instant`, the same explicit-timestamp idiom as `await_vblank`'s `now`)
drops entries older than one second before matching, so a driver that never
delivers the owed vblank can't freeze a reused CRTC. Pinned by six unit
tests including the ticket's pin (stale entry, synthetic clock past the
age, vblank for a new head on that CRTC settles), the exact-age boundary,
mixed ages, the unchanged prompt-arrival path, and VT-switch/error
clearing. The GPU tier needed nothing: `DrmCompositor`'s queue is private
and the residual is still documented in `scanout.rs` (verified, untouched).
No benchmark owed: the age-out runs on the vblank-arrival path over a list
that is empty in steady state, one timestamp compare per entry otherwise.

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
