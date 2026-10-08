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

## Resolution (2026-10-08, PR #519; trigger and PATH fixes 2026-10-08)

Landed as docs-only in `site/src/content/docs/scootbar/modules.md`, a new
"CPU, memory and pressure-stall recipes" subsection under the `exec` module:
CPU from `/proc/stat` (one-second delta, printed first, `sleep 60` loop),
memory from `/proc/meminfo` (`MemAvailable` with `MemFree` fallback), and the
event-driven PSI trigger (`some 150000 2000000` on `/proc/pressure/memory`,
swappable to `cpu`/`io`, blocked in `poll` with no timer). Each states what
it shows, its cost, and its reload behavior; bounds follows the module (lines
past 4096 bytes dropped, text cut at 256, controls to spaces), and the
recipes print only `printf`-formatted numbers. The first version documented
a 1 s trigger window the kernel refuses (`EINVAL` on the Asahi M2) and said
nothing about `python3` on the bar's `PATH`; both are fixed in this PR (the
page now names the multiple-of-2-s window rule, the `CONFIG_PSI`/`psi=1`
requirement, the `python3` prerequisite with its symptom box and PATH fix,
and why the pressure recipe stays Python while the per-minute loops stay
`awk`, with measured numbers). Evidence: snippet gate
(`test-snippets: ok`), `check-nix: ok`, `backlog check` with only the 3 known
pre-existing problems. Every recipe ran live on the Asahi M2 (kernel
7.1.13) under a real headless bar (`scoot --headless` + `scootbar daemon`,
own `XDG_RUNTIME_DIR`, scratch `HOME`):
trigger sweep `some 150000 {500000,1000000,1500000}` refused (22 `EINVAL`),
`2000000` accepted, `2500000` refused, `4000000` accepted, so the window
must be a multiple of 2 s; the fixed script printed `pressure ok`, then
under generated memory/CPU pressure (6 hogs, <=1.5 GB anon churn, 15–20 s;
memory `some avg10` rose 0.07 to 2.78) printed `stall memory 0.00` twice,
and the bar's `msg query` showed `stall memory 1.21` with a clean log;
CPU showed `CPU 100%` then `CPU 77%` (page shows the `CPU 42%` shape),
memory showed `MEM 71%` direct and `MEM 54%`/`MEM 62%` in the bar (page
shows the `MEM 75%` shape), with the no-`MemAvailable` fallback giving
`MEM 25%` and empty input giving no output; without `python3` on `PATH`
the bar logged `exec: python3: not found` (`exit status: 127`, backoff
1 s, 2 s, 4 s, ...) and the module kept its `...` placeholder. No code
changed, so no binary size or idle RSS/wakeup impact. `system-stats-decision`
stays open.
