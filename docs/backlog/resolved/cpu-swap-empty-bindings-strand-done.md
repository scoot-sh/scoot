---
title: "CPU-tier output swap with empty bindings can strand a headless output reporting live:true"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
---

# CPU-tier output swap with empty bindings can strand a headless output reporting live:true

Filed 2026-10-11. Serves **daily-drive**: a display that reports live while showing nothing.

## Resolution (2026-10-28)

Cause, read from the source: after an empty restore the next attempt's
binding snapshot is empty, and the CPU `swap` bound with `rebind`, whose
`retain_attached` dropped every fresh head no binding claimed. The new
`Tty` was already built `active` on the strength of the proof, so a swap
that left no head presenting reported `live: true` over the one stale
output (the last output is never removed), and the only way back was the
re-probe adding a head, which can refuse. `swap_scanout` (GPU) never had
the hole.

Fix, CPU `swap` only: bind by connector without dropping unclaimed heads,
route those through `Change::Added`, then run the same downgrade rule the
GPU swap uses (`tty/swap_bind.rs`): no presenting head means `active =
false`, and a swap with no heads at all also holds the primary. Zero- and
one-output edges are pinned in `tty/swap_bind/tests.rs`.

Honest limit: the trigger (a lost master race on a vacant device right after
an empty restore) cannot be produced on the dev VM and the M2 was off, so
the strand itself was not reproduced end to end; the binding and downgrade
decisions are pinned headlessly and the wiring is unexercised.

## The gap

On the CPU tier, a seat loss followed by a swap with empty bindings can leave
one headless output reporting `live: true` (the last-output refusal blocks its
removal) until the next successful seat win heals it. Found while verifying
the GPU-tier seat reconnect (#551); the win now heals it cleanly with no
duplicate outputs, but the source is not fixed. Evidence: the PR #551
implementer's report, "CPU follow-up #4", and the review's (b2) run on the M2.

## What to do

Reproduce on the CPU tier (dumb buffers), find why the swap with no bindings
keeps the stranded output `live`, and make `live` report the truth or remove
the stale output. Pin the zero- and one-output edges.

## Not in this ticket

The GPU-tier reconnect, which merged in #551.
