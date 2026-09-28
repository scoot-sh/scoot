---
title: "--nested confirms a session lock when the blanked frame is drawn, not when the host has it — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--nested`: confirm `locked` once the blanked frame reaches the host — RESOLVED

RESOLVED 2026-09-28 (PR number filled in on create). `State::render`'s
nested tail no longer confirms on the draw: a drawn frame records through
`State::note_nested_frame` (the `--tty` wait's own `await_vblank` with no
flip to track, arming the same once-per-wait `LOCK_VBLANK_TIMEOUT`
deadline and the same shared one-shot timer, not a second one) and
`locked` waits for `FrameOutcome::host_committed` -- the frame the host
committed and flushed. A frame the host takes at once confirms with no
wait armed at all. A dropped read-back frame (both `wl_shm` buffers held,
unconfigured, size mismatch, no pool, refused flush) and an owed dma-buf
frame (no usable host buffer: every resize's first frame, sometimes its
second) wait for the owed hand-over (`State::note_nested_handed_over`,
gated on the draw having been recorded for this lock so an owed frame
from before the lock cannot confirm it), a re-render (every resolvable
skip re-arms one), or the fallback (`note_blank_timeout`), which confirms
anyway after one second rather than hanging the locker -- the bound is
load-bearing, and it is the same bound `--tty` uses. Pinned by eight unit
tests (`session_lock/tests/nested_confirm.rs`: dropped read-back sends no
`locked` but arms the fallback -- fail-first verified against
confirm-on-draw semantics; prompt commit confirms with no wait; owed
hand-over confirms; pre-lock owed frame refused; the one-second bound
pinned both sides; redraws keep the first bound; unlock cancels), with the
three dma-buf ones behind `gpu-scanout` like every other dma-buf suite.
The tail routing itself (`host.is_some()` into `note_nested_frame`) is
covered by construction, the `vblank_confirm.rs` disclaimer's mirror. No
benchmark owed: the per-frame delta while no lock waits is two
predicted branches (host presence never changes mid-session, `pending` is
almost always `None`) with no allocation and no clock read; the wait path
is once per lock with at most one timer insert. The hidden/minimised-host
timing half is not stageable on the dev VM (cage-headless has no
hide/minimise, and no ext-session-lock client exists there to close the
lock-timing loop), stated rather than fabricated; what is shown live is
that owed hand-overs really happen against a real host (the
`nested/gpu/tests/live.rs` resize suite runs green on the VM) and that the
worst case is bounded whatever the host does.

Filed 2026-09-24 by the review of PR #235 (nested dma-buf presentation).
Serves **daily-drive** (a nested session's lock screen) and, marginally,
computer use (an agent that locks and screenshots the host).

`State::render` confirms a pending `ext-session-lock-v1` lock on the first
frame that *draws* blanked (`frame.drew_a_frame`); only `--tty` waits for
the flip that shows it (`SessionLock::await_vblank`). Under `--nested` the
drawn frame is not always the shown one: a read-back frame is dropped when
the host holds both `wl_shm` buffers, and since PR #235 a dma-buf frame is
*owed* whenever no host buffer is usable -- which is every resize's first
frame and, while the chain grows, its second. So `locked` can reach the
locker a host round trip (or, with a host that stops releasing buffers
while hidden, arbitrarily long) before the host shows the blanked frame.
The host keeps showing the last unlocked frame meanwhile.

Not fixed in PR #235 because the obvious fix -- gate confirmation on
`FrameOutcome::host_committed` and confirm from the owed-frame hand-over
too -- is shared by the pixman read-back path and introduces a new wait
with no bound: a host that never releases (hidden, stalled) would leave the
locker waiting for `locked` forever, which `--tty` answers with a fallback
timer (`LOCK_VBLANK_TIMEOUT`). A fix needs the same shape: confirm on the
first blanked frame the host has (committed and flushed, or handed over
with nothing pending since), with a bounded fallback, and a suite that
drives a skipped blanked frame on both presentation paths.

Evidence to gather first: whether the window is ever more than one host
round trip on a real host (a hidden or minimised nested window under GNOME
or KDE is the likely long case).
