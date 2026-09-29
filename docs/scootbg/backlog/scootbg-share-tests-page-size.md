---
title: "scootbg share tests hard-code 4K page rounding, fail on 16K-page hosts (Asahi)"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# scootbg share tests hard-code 4K page rounding, fail on 16K-page hosts (Asahi)

Filed 2026-09-29 (PR #332 review). Serves **daily-drive** on 16K-page hosts (Apple Silicon under Asahi).

## The gap

Two `scootbg::share::*` tests hard-code 4K-page rounding: 1600x1000x4 bytes rounds to 6252 KiB on 4K pages and 6256 KiB on 16K pages (6256 vs 6252 pages in the failure). They pass on the dev VM and fail on `main` on the Asahi M2 box.

## What to do

Compute the expected size from `sysconf(_SC_PAGESIZE)` instead of the literal.

## Not in this ticket

Any product change: this is test hygiene only.
