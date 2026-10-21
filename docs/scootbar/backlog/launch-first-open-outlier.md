---
title: "scootlaunch: reproduce the one ~15 s first-ever panel open (or let it age out)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch: reproduce the one ~15 s first-ever panel open (or let it age out)

Filed 2026-10-21. Serves **daily-drive**. From the A/B bench
(dev/benchmarks/launcher-ab-2026-10.md, Caveats): one single first-ever
panel open took ~15 s to first frame; every later cold-cache open measured
35–150 ms across 90+ cold opens. Not reproduced, cause not isolated.

## The gap

A single ~15 s first open with no known cause. If it is real and reachable
(e.g. first-run fontconfig scan, icon-cache build, cold dylib pages), every
new user pays it once. If it was a one-box fluke, it ages out.

## What to do

- Reproduce on demand: fresh `$HOME` (no index cache, no fontconfig cache),
  cold page cache (drop caches with root, which the bench could not), time
  exec-to-first-frame; bisect font scan vs icon load vs index build.
- If reproduced and fixed, note the fix and close. If it does not reproduce
  across a release cycle of daily use, close as aged out.
- Until either, `desktop.launcher.daemon = "fuzzel"` stays one line away
  (the flip's condition 2).

## Not in this ticket

The fuzzel mouse no-op under scoot (bench M5): keyboard flows unaffected,
not isolated whether compositor or fuzzel, not ours to root-cause.
