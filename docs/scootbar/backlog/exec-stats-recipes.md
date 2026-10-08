---
title: "Documented exec recipes for CPU/memory/pressure readouts"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# Documented exec recipes for CPU/memory/pressure readouts

Filed 2026-10-07, split out of `system-stats-decision` (which stays open as
the standing decision record). Serves **daily-drive**: today a user who
wants a CPU readout has the mechanism but no documented recipe.

## The gap

The decision says "Serve them with the `exec` module ... Ship documented
recipes." The `exec` module exists and is documented
(`site/src/content/docs/scootbar/modules.md` "Button, push and exec
modules": streaming, no `interval` key by design, backoff restarts), but no
recipe ships anywhere (checked 2026-10-07: no CPU/meminfo/pressure mention
in `site/src/content/docs/scootbar/`, only an incidental "0.03 CPU-seconds"
in a measurement note).

## What to do

Document one-line `exec` recipes on the site beside the `exec` module (to
the docs-bar standard: copy-paste correct, tested by the snippet gates):
CPU from `/proc/stat` (print first, `sleep 60` loop, so the cost is visible),
memory from `/proc/meminfo`, and the event-driven PSI stall trigger from
`/proc/pressure/{cpu,memory,io}` (no timer at all) the decision names. Bound
lengths, strip control characters, keep the interval coarse and say the cost.

## Not in this ticket

Any built-in stats module (refused by the decision); the decision record
itself (`system-stats-decision.md`, stays open).
