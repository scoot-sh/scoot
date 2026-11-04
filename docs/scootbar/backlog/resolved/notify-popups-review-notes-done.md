---
title: "scootnotify: popup review notes (libm sentence, click coalescing, replace unpauses, late seat)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-04"
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

## Resolution

Landed in one PR:

- Docs: the site's scootnotify page now says "glibc, libm and libgcc_s".
- Click coalescing: presses queue (oldest first, at most 8) instead of
  overwriting one slot. This also fixed a stall the ticket did not name: a
  press read while an earlier one waited for its token sat in the slot until
  the next Wayland event, so `dispatch` now drains the queue in the same turn.
- Replace on a paused card: `Clock::restart` keeps a paused clock paused and
  parks the fresh timeout, so a card under the pointer (or idled) does not
  expire on a replace. Decision: hover survives a replace.
- Late seat: the idle notification arms when the seat or the idle notifier
  global arrives, not only at connect (`arm_idle` is idempotent). No
  automated test: headless scoot never advertises a global late.
