---
title: "Wi-Fi picker can list a network twice after a scan handover"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Wi-Fi picker can list a network twice after a scan handover

Filed 2026-10-04 from PR #428's round-2 review (R1, R2). Serves
**daily-drive** (polish: a doubled row in the network picker).

## The gap

`crates/scootbar/src/modules/network/mod.rs` tracks a scan refill with
`scan_reset`, `live` and `spent`. Two orderings leave a queued refill without
the reset it is owed, so it appends onto a list that already holds some of
the same networks:

- **R1:** a retarget scan D1 is in flight with pages pending; one page lands;
  the menu opens (arms the reset, hands it to unspent D1, queues D2); D1's
  next page consumes the reset; D2 then appends its full scan after it. The
  later page's network is listed twice, and with more than `MAX_SCAN`
  networks the doubled slots can push tail entries out.
- **R2:** the same setup, but the kernel refuses D1: `drop_genl` cancels the
  reset D1 owned while D2 stays queued with nothing owed, so D2 appends onto
  the stale pre-arm list.

Both are transient (the next arm or re-dump clears at its first page), never
pick the wrong network on `connect` (the snapshot and its staleness check do
not depend on uniqueness) and never wedge. R1 needs an arm between two pages
of one netlink burst; R2 additionally needs a refused scan.

## What to do

Make the queued refill own its reset whenever the in-flight scan consumes or
cancels one it was handed. Do not simply drop D2 at consume: that breaks the
zero-page heal (D1 ending with no pages and D2 refilling the cleared list).
Add a test for each ordering above, failing before the fix.

## Not in this ticket

R4 (an empty owning dump with nothing queued stays empty until the next
wireless event): pre-existing, heals on the next event.

## Resolution (2026-10-08, PR #507)

Fixed as the ticket recommended: the queued refill owns its reset
whenever the in-flight scan consumes or cancels one it was handed. New
`Nets::scan_queued` helper; the scan page, `done_genl` terminator and
`drop_genl` refusal hand the handed reset on to the queued refill
instead of spending or cancelling it. The zero-page heal stands (D1
ending with no pages still clears the stale list at its terminator, and
the refill still fills it). No new struct fields, fds, timers, threads
or hot-path allocations.

Tests, each failing before the fix and passing after (Asahi M2):

- `a_handover_between_pages_does_not_double_a_network` (R1): before,
  the menu showed `Elsewhere / FarAway / Elsewhere`; after,
  `FarAway / Elsewhere`.
- `a_refused_handover_scan_keeps_the_queued_refill_s_reset` (R2):
  before, the menu showed `FarAway / FarAway / Elsewhere`; after,
  `FarAway / Elsewhere`.

Also added the test-only `fake::error_seq` helper so a refusal can
answer a specific in-flight dump. Full `scootbar` nextest suite green
(1387 passed, 0 failed; the one sway-gated clock test needs sway, which
the box has none of — CI covers it). Ratchet: release file unchanged at
2,364,128 B, `.text` +416 B (+0.02%), all other sections identical;
idle wakeups level with base (2–3 per 20 s both sides); the `.text`
delta is reported in the PR for the maintainer, covered by no waiver.
