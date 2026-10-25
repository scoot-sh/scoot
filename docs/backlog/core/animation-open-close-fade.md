---
title: "Slide animation follow-up: open/close fade"
status: "open"
area: "core"
priority: "low"
blocked: null
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
