---
title: "Verify the corner_radius reload repaint on a real GPU scanout output"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Verify the corner_radius reload repaint on a real GPU scanout output

Filed 2026-11-02 from the review of PR #868. Serves **daily-drive**: a
radius change in a live session must not leave stale corner or margin
pixels on the GPU tier most desktops will run.

## The gap

PR #868 makes a `corner_radius` reload repaint the whole output. On the
dumb-buffer tiers it resets the output's `OutputDamageTracker`
(`Backend::reset_damage`). On the GPU scanout tier, `DrmCompositor` owns its
own tracker, so the reload instead arms one composite-only frame and drops
the swapchain slots on each output's scanout presenter (`arm_force_composite`
plus `invalidate_scanout`, the pattern `force_scanout_composite` uses for
stale captures). That arm was derived from the pinned Smithay fork's source
and pinned only by a test that the reload visits every output's presenter.
No run has put a real GPU scanout output through a radius reload, because
the dev VM has no GPU and the arming needs a `Tty`.

## What to do

On a real GPU output (the Asahi M2 with `--tty --renderer gpu`, switched in
from the normal session the way `docs/roadmap` describes), run a client that
draws outside its geometry. Reload with `corner_radius` 0 to 12, 12 to 0 and
12 to 6, screenshot the corners and margin after each, and record the
result. A stale pixel is a bug: file it with the screenshot.

## Not in this ticket

The dumb-buffer tiers (covered by the regression test in #868) and any other
reload key (the review found none share the bug).
