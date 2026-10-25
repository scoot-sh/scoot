---
title: "Slide animation follow-up: workspace-switch slide"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Slide animation follow-up: workspace-switch slide

Filed 2026-10-25. Serves **daily-drive** (workspace switches currently snap while column scrolls slide).

## The gap

Switching workspaces moves every tiled window at once; the per-window
x-slide (`crates/scoot/src/compositor/animation.rs`) animates each move
individually rather than the viewport as a unit, which reads as many
windows sliding rather than one screen turning over.

## What to do

Decide whether a workspace switch should slide the whole strip as one
viewport (single offset for the output) or keep per-window slides, and
implement the chosen shape behind the `[animation]` gate with the same
zero-cost-when-off bar (no timer/idle wakeups when idle, damage-limited,
no client re-commit). Pin: multi-output, lock, reload-disable, empty and
single-window workspaces.

## Not in this ticket

Open/close fade, vertical slides, springs (siblings).
