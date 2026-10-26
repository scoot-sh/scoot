---
title: "Slide animation follow-up: workspace-switch slide"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-25"
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

## Resolution (2026-10-26, PR to be opened)

Landed as `feat(scoot): slide the incoming strip on workspace switches`.
Chose the single-offset viewport shape (incoming-only): when the active
workspace index on an output changes and the new workspace shows a tiled
window, that output slides its incoming strip in from the output edge
(`+width` for a higher index, `-width` for a lower one) with the session's
`duration_ms`/`easing`. Outgoing windows are not drawn (unmapped as before);
drawing them would double the frame's elements and damage the full viewport
per frame for a cross-slide few asked for. `scoot-core` untouched; IPC and
hit-testing keep the target; one `Instant` per output frame; no allocation
on the per-frame path; lock and reload-disable snap; empty workspaces snap;
single-window is one window with the shared offset; multi-output switches
stay per-output. 8 new tests in `animation/tests.rs` cover the pin list
(delta shape, direction, empty/single, multi-output independence, lock,
disable, reload-disable, prune/idle). Measured on the CPU renderer
(headless, pixman, release, dev VM): see the PR report for before/after
`render_frame_cost` numbers; disabled/idle cost unchanged (empty-branch
only).
