---
title: "The lightest bar: the release gate against yambar and waybar"
status: "open"
area: "scootbar"
priority: "high"
blocked: "workspaces-module"
---

# The lightest bar: the release gate

Filed 2026-09-29. Serves the project's "low memory and CPU is the name of the
game". Modeled on scootbg's
[`lightest.md`](../../scootbg/backlog/lightest.md).

v1 is not released while any competitor beats scootbar, beyond a noise
margin, on any row that applies to both. After v1, every PR that touches
drawing, the event loop or a module re-runs the benchmark and must not
regress beyond the margin against the last published numbers.

## Rows

The baselines from [baselines-and-spikes](baselines-and-spikes.md), on the
same machine and outputs, for the v1 bar (clock and workspaces): idle RSS and
PSS, idle wakeups per minute (target: one, the clock), CPU over a fixed window
while switching workspaces, peak memory, binary size, startup to first frame.
Add a row per module as it lands (battery, volume, network) against the same
competitors' equivalent modules.

Published in `docs/scootbar/README.md` with the method, and a benchmark script
under `scripts/` like `scripts/scootbg-bench`.

## Rules

- A loss on any row is a finding to fix, or a waiver written down by the user
  with the class it covers, as scootbg's was.
- Measure release builds only (`lto = "fat"`, `panic = "abort"`).
- Compare after idle settles; say how it was detected.
