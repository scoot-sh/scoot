---
title: "The resource ratchet: every milestone measured against the last one and the competitors"
status: "open"
area: "scootbar"
priority: "high"
blocked: "baselines-and-spikes"
milestone: "ongoing"
---

# The resource ratchet

Filed 2026-09-29; reframed the same day from a single release gate to a
ratchet, so the bar can ship in small steps. Serves the project's "low memory
and CPU is the name of the game". Modeled on scootbg's
[`lightest.md`](../../scootbg/backlog/lightest.md), but applied at every
milestone, not once at the end.

## The rule

A milestone is not done until its numbers are published and:

1. **No row regresses** beyond the noise margin against the previous
   milestone's published numbers, except where the new module adds a row that
   did not exist (then that row is measured against the competitors' equivalent).
2. **At the milestone's own scope, no competitor beats scootbar** beyond the margin
   on any row that applies to both. Clock and workspaces are compared with
   yambar and waybar showing exactly those modules; battery is compared when
   the battery module lands, and so on. A loss is a finding to fix, or a
   waiver the user writes down with the class it covers, as scootbg's was.
3. **The size of the codebase and its dependency count are rows too**: lightweight
   bars have died of maintainer burnout (yambar's own README says it is no longer
   developed), so keeping the bar small enough for one person is a measured
   property, not a hope.

## Rows

From [baselines-and-spikes](baselines-and-spikes.md), on the same machine and
outputs: idle RSS and PSS, idle wakeups per minute (target for the first
milestones: one, the clock), CPU over a fixed window while switching
workspaces, peak memory, stripped binary size, installed closure size
([nix-package](nix-package.md) keeps fonts out of it), startup to first frame,
plus lines of code and direct dependency count.

Also **a multi-day soak**: RSS and fd count sampled over days with the bar
running through suspend/resume, DPMS wake and hotplug, since the bugs people
report in other bars are growth and CPU loops, not one-frame costs
([robustness-and-limits](robustness-and-limits.md)). Neither number is found
anywhere public for the competitors (no measured RSS or wakeup benchmarks turned
up in research), so the published comparison is itself a contribution.

Published in `docs/scootbar/README.md` with the method, by
`scripts/scootbar-bench` (reusing `scripts/scootbg-bench`'s runner), with each
milestone's table kept, not overwritten.

## Rules

- Release builds only (`lto = "fat"`, `panic = "abort"`).
- Compare after idle settles; say how it was detected.
- After a milestone, every PR touching drawing, the event loop or a module re-runs
  the benchmark.
