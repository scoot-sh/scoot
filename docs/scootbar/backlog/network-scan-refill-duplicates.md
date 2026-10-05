---
title: "Wi-Fi picker can list a network twice after a scan handover"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
