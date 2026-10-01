---
title: "Decision: CPU, memory, temperature and disk are not built in (they need polling)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "ongoing"
---

# Decision: no built-in system-stats modules initially

Filed 2026-09-29 as a decision record, so the question has an answer when it
is asked. Serves the lightness goal: "not polling where we can avoid it".

`/proc/stat`, `/proc/meminfo`, thermal zones and `statvfs` are **state with
no change notification**: the only way to show them live is to sample on a
timer. A bar that samples CPU every second wakes a process every second
forever, and that is precisely what the resource ratchet measures competitors
losing on.

## Recommendation

- **Do not build them in at first.** Serve them with the [`exec` module](resolved/exec-push-button-modules-done.md):
  a user who wants a CPU readout writes a one-line script with the interval
  they choose, and pays for it knowingly. Ship documented recipes.
- If a built-in is ever added, it is opt-in, off in the smallest build, driven
  by **one shared timer** that runs only while some visible module needs it, with
  the interval in config (default coarse), and measured in the
  [resource ratchet](lightest.md) as its own row.
- The one event-driven source is **PSI** (`/proc/pressure/{cpu,memory,io}`),
  which supports `poll(2)` with a stall-threshold trigger: a "memory pressure
  high" indicator can be fully event-driven. Worth a spike if pressure, rather
  than raw usage, is what people want to see; it needs no timer at all.

## Not in this ticket

Any implementation. Revisit if a user asks for a stat that `exec` cannot serve
cheaply.
