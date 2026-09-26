---
title: "`world.arrange()` runs per output per frame, including no-damage passes"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# `world.arrange()` runs per output per frame

Filed from `dumb-tier-damage-history-desync.md` (adjacent, priced
separately -- not fixed there). Not a correctness gap: an optimisation.

## What is wrong

`render::draw_frame_with` calls `state.world.arrange()` once per frame
per output (`render.rs`: the `arrangement` binding ahead of the bind),
locked or not, damaged or not. A no-damage pass on a clean output -- now
cheaper after the desync fix (no composite, no present, no flip) -- still
pays a full core arrangement for that output's strip.

The per-call cost itself under floods is
[per-client-toplevel-cap](./per-client-toplevel-cap.md)'s subject (linear
at measured shapes, quadratic shapes found and fixed ad hoc); this ticket
is only about the call *count*: one arrange per output per frame even when
that output will draw nothing. On a quiet multi-output session the clean
screens re-arrange every time any screen animates.

## Fix direction (design open)

Cache the arrangement per output and reuse it while neither the core's
layout inputs (windows, workspaces, sizes, focus) nor the frame's inputs
(scale, output geometry, lock state) changed -- or hoist the arrange out
of the per-output walk (one arrange per frame, shared by every output's
element gathering). Needs: proof that no per-output frame input leaks into
the arrangement (pointer/output-local state must not ride along), and a
benchmark showing the saving matters next to a no-damage pass's remaining
cost (element gathering still walks per output). Deliberately not done
with the desync fix: a stale-arrangement bug shows wrong pixels with no
error, the worst kind.
