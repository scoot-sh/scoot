---
title: "scootnotify: popup review notes (libm sentence, click coalescing, replace unpauses, late seat)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootnotify: popup review notes

Filed 2026-10-19 from the independent review of PR #683. Serves **daily-drive**. None blocked the merge.

## What to do

- **Docs fix:** `site/src/content/docs/scootnotify/index.md` says the binary links "nothing beyond glibc and libgcc_s"; `ldd` also shows `libm.so.6` (the CI gate allows it). Say "glibc, libm and libgcc_s".
- **Click coalescing:** a second press inside the 200 ms activation-token wait overwrites the first, so one click is lost. Keep single-slot storage but document it, or queue.
- **Replace on a hovered card:** `Clock::restart` clears `paused`, so a `replaces_id` while the pointer rests on the card restarts its clock running. Decide whether hover should survive a replace.
- **Late seat:** the idle notifier is armed once at connect; a seat that appears later never re-arms it. Safe degradation (clocks keep running, one warning); re-arm on a new seat global.

## Docs (same PR)

The libm sentence is the docs change; the other items change site text only where behavior changes.
