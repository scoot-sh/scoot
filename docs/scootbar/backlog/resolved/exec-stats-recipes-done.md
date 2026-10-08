---
title: "Documented exec recipes for CPU/memory/pressure readouts"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
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

## Resolution (2026-10-08, PR #519)

Landed as docs-only in `site/src/content/docs/scootbar/modules.md`, a new
"CPU, memory and pressure-stall recipes" subsection under the `exec` module:
CPU from `/proc/stat` (one-second delta, printed first, `sleep 60` loop),
memory from `/proc/meminfo` (`MemAvailable` with `MemFree` fallback), and the
event-driven PSI trigger (`some 150000 1000000` on `/proc/pressure/memory`,
swappable to `cpu`/`io`, blocked in `poll` with no timer). Each states what
it shows, its cost, and its reload behavior; bounds follows the module (lines
past 4096 bytes dropped, text cut at 256, controls to spaces), and the
recipes print only `printf`-formatted numbers. Evidence: snippet gate
(`test-snippets: ok`), `check-nix: ok`, `backlog check` with only the 3 known
pre-existing problems; each recipe executed (CPU 50% on a synthetic delta,
MEM 75% on synthetic meminfo, PSI script `py_compile` clean with the
missing-PSI path printing a flushed `pressure n/a` and blocking). No code
changed, so no binary size or idle RSS/wakeup impact. `system-stats-decision`
stays open.
