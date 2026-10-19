---
title: "Spike: hold non-critical cards under a fullscreen window via a `top` layer surface"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-popups"
milestone: "M7"
---

# Spike: a fullscreen hold for scootnotify

Filed 2026-10-12 in the #579 fix round. It replaces the first draft's foreign-toplevel
machinery with a cheaper idea to measure. Serves **daily-drive**. Status research; v1 shows
cards over fullscreen windows, as mako does today.

## The idea

scoot withholds frame callbacks from a `top` layer surface that has committed a buffer, while an
opaque fullscreen window covers its output (`crates/scoot/src/compositor/layer_shell/occlusion.rs:15-26,96-113`).
scootnotify's expiry clock starts at the first frame callback, so non-critical cards on a `top`
surface would be held for free. Critical cards stay on the `overlay` surface.

## What to measure

- a translucent fullscreen window, where the callback is served and the clock starts while the
  card is hidden;
- the cost of a second surface;
- behavior on sway (the rule is scoot-specific).

## Acceptance

A recorded go or no-go in this entry, and the design updated.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/configure.md` with the fullscreen-hold behavior and its option, if the spike ships one; else a dev/ note only. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.
