---
title: "--nested confirms a session lock when the blanked frame is drawn, not when the host has it"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# `--nested`: confirm `locked` once the blanked frame reaches the host

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
