---
title: "xwayland refused_cost test fails: a refused managed window costs the server 273 fds"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-26"
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

## Resolution (2026-10-26)

Not a product leak: the test measured the wrong thing. Evidence, dev VM
(`/dev/shm/wt-bl1`, `origin/main` = `3364900c6`, single-test runs under both
runners):

- `nextest` alone: `256 buffers and 273 fds` (stable across 3 runs); `cargo
  test` alone: identical `256 buffers and 273 fds`. Buffers exact, fds +17.
- Debug inventory at the failure point: `held_by = 273`,
  `identities_of = 273`, `/proc/self/fd` total 278 with 258 `memfd`, and after
  `in_flight` sweep `total_live_all = 256, server_buffers = 256,
  server_fds = 256`. The 17 extra records are dead: their fds already closed
  (proc shows ~256 live memfds, not 273). Soft limit 65536, server bound 4096.
- The refused-manage path (`manage.rs::map_x11_window`) returns before
  `set_mapped`, allocating nothing; buffers confirm refused windows cost
  nothing. The dead records are resized managed windows' old pools: each
  resize allocates a new pool while Smithay's dropping thread closes the old
  fd a moment later, and whether the next allocation reuses the number decides
  whether the record is forgotten (`fd_arrived`) or lingers dead until a
  sweep. `held_by` is documented as an upper bound for exactly this reason,
  and production refuses only on a fresh sweep (`admit`). The test asserted on
  the unswept bound, so it counted timing as cost; CI passed on luckier
  reuse timing.

Fix (test-only, cap kept): the test now maps the 128 first, records a swept
baseline (`swept_held_by`: new `#[cfg(test)]` helper on `ClientFds` that sweeps
with the real checks, plus a `swept_cost` reader), asserts the baseline pins
the per-window cost (`<= 256 + 8`), maps the 72 refused, asserts none admitted
and the swept cost grew by at most 8 over its own baseline. Verified: 5/5
`nextest` loops green, `cargo test` 3/3 green, all 187 `xwayland` `nextest`
tests green, `client_fds` 38 green, `clippy -D warnings` and `fmt --check`
clean. The `look_golden_screenshots` failures in a full `-p scoot` run (6/7)
reproduce identically on unmodified HEAD and are out of scope for this ticket.
