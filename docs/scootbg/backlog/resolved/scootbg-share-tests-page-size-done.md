---
title: "scootbg share tests hard-code 4K page rounding, fail on 16K-page hosts (Asahi)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-09-30"
---

# scootbg share tests hard-code 4K page rounding, fail on 16K-page hosts (Asahi)

Filed 2026-09-29 (PR #332 review). Serves **daily-drive** on 16K-page hosts (Apple Silicon under Asahi).

## The gap

Two `scootbg::share::*` tests hard-code 4K-page rounding: 1600x1000x4 bytes rounds to 6252 KiB on 4K pages and 6256 KiB on 16K pages (6256 vs 6252 pages in the failure). They pass on the dev VM and fail on `main` on the Asahi M2 box.

## What to do

Compute the expected size from `sysconf(_SC_PAGESIZE)` instead of the literal.

## Not in this ticket

Any product change: this is test hygiene only.

## Resolved 2026-09-30

`buffer_kb` in `crates/scootbg/tests/share.rs` was the one place: it rounded a
buffer to 4 KiB pages. It now rounds to the host's page size, read from the
auxiliary vector (`AT_PAGESZ` in `/proc/self/auxv`) with `std` alone, so there is
no new dependency and no `sysconf` binding (the ticket's suggestion; scootbg has
no `libc` crate, and `rustix::param::page_size()` would need another feature).
1600x1000x4 bytes is 6252 kB on 4 KiB pages and 6256 kB on 16 KiB pages, which
are the two values the failure showed.

Evidence, on the Asahi M2 (16384-byte pages): with a current `scoot` beside the
tests, `cargo nextest run -p scootbg -p scootbg-mem -p scootbar` was 870 passed,
**2 failed** (exactly `scootbg::share outputs_of_one_size_share_one_image_on_scoot`
and `full_size_colors_never_write_into_pixels_another_output_shows`, 6256 against
6252); with the fix it is 871 passed, 0 failed, 2 skipped. On 4 KiB hosts the
formula is unchanged (6252), which CI covers.

A gotcha found on the way: those tests run against whichever `scoot` sits
beside them, and a stale `target/debug/scoot` (here from nine days earlier) made
**eight more** tests fail that have nothing to do with pages (scale, color and
config cases in scootbg and scootbar) and passed once `scoot` was rebuilt.
`cargo build -p scoot` first.
