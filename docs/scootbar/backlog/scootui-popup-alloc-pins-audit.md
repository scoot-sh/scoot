---
title: "Audit the popup and icon zero-allocation pins that ran without a counting allocator"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# Audit the popup and icon zero-allocation pins that ran without a counting allocator

Filed 2026-10-18. Serves **daily-drive (the bar's zero-allocation bar is a stated standard)**.

## The gap

PRs #670 and #671 found that `scootui`'s lib test binary had no counting `#[global_allocator]`, so `scootbg_mem::count_allocations` always read 0 there. #670 installed one (`crates/scootui/src/lib.rs`, `cfg(test)`). The older pins that ran under it before then (popup tip/tests, icon cache and raster) were vacuous until now; they pass today but have never been mutation-proved.

## What to do

For each `count_allocations` pin in scootui (popup, icon), inject a `format!` into the measured path, see it fail, revert, and record it. Fix any pin that turns out to measure nothing.

## Not in this ticket

scootbar's own pins: its binary crate installs the allocator and its pins were already real.
