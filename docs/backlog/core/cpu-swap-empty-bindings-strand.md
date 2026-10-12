---
title: "CPU-tier output swap with empty bindings can strand a headless output reporting live:true"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# CPU-tier output swap with empty bindings can strand a headless output reporting live:true

Filed 2026-10-11. Serves **daily-drive**: a display that reports live while showing nothing..

## The gap

On the CPU tier, a seat loss followed by a swap with empty bindings can leave
one headless output reporting `live: true` (the last-output refusal blocks its
removal) until the next successful seat win heals it. Found while verifying
the GPU-tier seat reconnect (#551); the win now heals it cleanly with no
duplicate outputs, but the source is not fixed. Evidence: the PR #551
implementer's report, "CPU follow-up #4", and the review's (b2) run on the M2.

## What to do

Reproduce on the CPU tier (dumb buffers), find why the swap with no bindings
keeps the stranded output `live`, and make `live` report the truth or remove
the stale output. Pin the zero- and one-output edges.

## Not in this ticket

The GPU-tier reconnect, which merged in #551.
