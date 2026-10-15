---
title: "xwayland refused_cost test fails: a refused managed window costs the server 273 fds"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# xwayland refused_cost test fails: a refused managed window costs the server 273 fds

Filed 2026-10-15, from the look-layer-scoot review (#613). Serves
**daily-drive**: leaked fds on a window path are a slow session death.

## The gap

`xwayland::tests::refused_cost::a_refused_managed_window_costs_the_server_nothing`
fails: it counts 273 fds against a cap of 264
(`crates/scoot/src/compositor/xwayland/tests/refused_cost.rs`). Pre-existing,
not from #613: the file, the ledgers it counts, and every code path it
executes are untouched by that diff, and it fails deterministically with or
without load. Not verified against `main` on the same VM (a main build did
not fit on that VM's disk); re-verify there first.

## What to do

Find what holds the extra ~9 fds on the refused-manage path (reaper queue,
unreaped child, double-counted ledger), fix it, and keep the cap. Pin with
the existing test; record the fd inventory before/after.

## Not in this ticket

Any look-layer or reload behavior.
