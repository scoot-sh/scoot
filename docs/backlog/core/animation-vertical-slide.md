---
title: "Slide animation follow-up: vertical slide"
status: "open"
area: "core"
priority: "low"
blocked: null
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
