---
title: "scootlaunch: detach helper cwd check; watch the toggle during spawn"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch: detach helper cwd check; watch the toggle during spawn

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #676. N1: the detach helper applies a relative Path= unchecked (the direct path falls back to $HOME): pass the resolved absolute cwd into the spec. N2: toggle-close is not watched during the spawn/scope step (waits out the <=500 ms bound); poll accept_close there if it can be done without a thread.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if user-visible; otherwise say so in the PR.
