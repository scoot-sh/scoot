---
title: "Slide animation follow-up: vertical slide"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-25"
---

# Slide animation follow-up: vertical slide

Filed 2026-10-25. Serves **daily-drive** (stacked windows in a column currently snap vertically).

## The gap

The slide animates x only (`animation.rs::Slide { from_x, to_x }`;
width/height snap). Windows stacked in one column change y when a
sibling opens/closes, and that move jumps.

## What to do

Extend the slide to y (or add a separate vertical pass) with the same
interpolation, retarget-from-visual and damage discipline, still with no
client re-commit. Measure the frame cost against the horizontal-only
slide on the CPU renderer before merging. Pin: same edge list as the
horizontal slide (zero/one window, retarget, close/output/workspace,
fullscreen/maximize/floating, lock, reload, resize, extreme sizes).

## Not in this ticket

Open/close fade, workspace-switch slide, springs (siblings).

## Resolution (2026-10-26, PR #749)

Landed as `feat(scoot): slide windows vertically as well as horizontally`.
`Slide` gained `from_y`/`to_y`; the frame path (windows, rings, clip, on
every backend including scanout and capture) reads one `visual_delta` scan
per placement per frame; `update_slides` starts, retargets and drops y
slides exactly like x. No new option, no CLI or protocol change;
`scoot-core` untouched. 11 new tests in `animation/tests.rs` cover the pin
list on y. Measured on the CPU renderer (headless, pixman, release, same
VM): 4.9ns per placement in flight vs 3.1ns before (+~1.9ns, one extra
lerp), idle/disabled unchanged at 0.24ns/call.
