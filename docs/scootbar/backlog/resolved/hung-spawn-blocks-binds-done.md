---
title: "Hung spawned children block every other exec binding until the bar restarts"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
---

# Hung spawned children block every other exec binding until the bar restarts

Filed 2026-10-16 from the looks-picker review (PR #637). Serves **daily-drive**:
a hung app reload that strands a `scoot look apply` child can wedge the bar's
other bindings with no recourse but a restart.

## The gap

The spawner runs at most 8 launched commands at once
(`crates/scootbar/src/spawn.rs`: `MAX_CHILDREN`); one past it is refused,
never queued. A launched child otherwise lives as long as it likes: the bar
does not supervise it. Eight hung `scoot look apply` children — each waiting
out an app's bounded reload — fill the table, and every other exec binding
(a volume key, a launcher, the next apply) is refused until the bar
restarts. The refusal is loud (a log line and, for `invoke`, an error), but
there is no way to clear the table short of a restart: no timeout, no kill,
no queue.

## What to do

Decide what a stuck table should do, without growing a supervisor. Options:
refuse with the hung command named and its age (so the log says what to
kill by hand); a bounded wait per spawn (queue one deep, not eight); or a
way to reap a named child through `msg`. Pin the edge cases first: a child
that exits between the check and the kill, a `msg`-driven kill racing a
natural exit, and what `invoke` answers while the table is full. Measure
before and after: the exec tests already pin spawn/reap timing.

## Not in this ticket

Redesigning the spawner, supervising children, or timing out applies: the
looks picker stages one apply per pick and reports the refusal, which is
enough until this ticket lands. Killing hung children from the bar is
explicitly out: the bar never signals another process today, and that
needs its own design.

## Resolution

Took the first option only. A refused spawn (log line and the `invoke`
error) now names every running command oldest first: program basename (cut
at 24 bytes, control characters shown as `?`, no arguments), pid and
monotonic age, so the user can `kill` the hung one by hand. The table is
re-reaped before the refusal is built, so a child that just exited frees
the slot instead of being named.

Declined, on purpose:

- **A bounded wait or queue.** Reaping is already the one place slots free,
  so a one-deep queue would be small, but a queued command fires at an
  arbitrary later time (a volume key that lands a minute after the press),
  which is worse than a loud refusal.
- **A `msg` kill of a named child.** The bar never signals another process
  (the ticket's own exclusion); it needs its own design, with the
  kill-versus-natural-exit race it brings. Park it until a user asks.
