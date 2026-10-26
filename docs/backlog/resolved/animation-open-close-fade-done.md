---
title: "Slide animation follow-up: open/close fade"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-26"
---

# Slide animation follow-up: open/close fade

Filed 2026-10-25. Serves **daily-drive** (polish for opening/closing windows; the slide ships without it).

## The gap

New windows appear and closed windows disappear instantly while existing
tiled windows slide (`crates/scoot/src/compositor/animation.rs`: new
windows snap, closed windows lose their slide). niri fades and scales
windows on open/close; scoot does neither yet.

## What to do

Add an optional open/close fade (and possibly scale) for tiled windows,
behind the same `[animation]` gate or a second knob, reusing the slide's
frame scheduling (frames only while in flight, idle when done) and damage
discipline (old+new rects only). Pin: zero/one window, retarget mid-fade,
close-during-open, output removed, lock, reload-disable snaps, no client
re-commit.

## Not in this ticket

Workspace-switch slide, vertical slides, springs (siblings).

## Resolution

Landed as `feat(scoot): fade tiled windows in on open and out on close`.
Chose fade-only (no scale) behind the same `[animation]` gate (no second
knob): a scale would resample every fading window per frame and tempt a
re-commit, while a fade is one alpha multiply per window with the damaged
rect never moving; the gate already carries the zero-cost-when-off bar and
all siblings reused it. Open windows fade `0 → 1`; closed windows keep
their last buffers drawn while fading `1 → 0`, retained as `Arc` clones
bounded by `MAX_CLOSING_FADES` (8 windows × ≤500 ms; the 9th close snaps
the oldest away). Same duration/easing, frame scheduling, damage
discipline and no-client-recommit bar as the slide; `scoot-core`
untouched; IPC and hit-testing keep the target; one `Instant` per output
frame; no per-frame allocation. 16 new tests in `animation/tests.rs`
cover the pin list (fade math, zero windows, retarget mid-fade,
close-during-open, output removed, lock, reload-disable, no target move
or commit). Measured on the CPU renderer (headless, pixman, release, dev
VM): see the PR report for before/after `render_frame_cost` numbers;
disabled/idle cost unchanged (empty-branch only).
