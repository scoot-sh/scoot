---
title: "scootbar links the same allocator: check it for the glibc 2.44 huge-page arena"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootbar links the same allocator: check it for the glibc 2.44 huge-page arena

Filed 2026-10-28. Serves **daily-drive**: scootbar is a resident daemon with a small-footprint goal.

## The gap

scootbg's settled daemon held 2.5 MB of anonymous memory on the aarch64 dev VM (glibc 2.44, kernel THP mode `madvise`): the decode thread's malloc arena was one resident 2 MiB huge page. `GLIBC_TUNABLES=glibc.malloc.hugetlb=0` gave 0.6 MB. scootbg now sets `PR_SET_THP_DISABLE` in `daemon::run`. scootbar shares the `scootbg-mem` allocator and any worker-thread arena, and was not measured.

## What to do

Measure scootbar's anonymous PSS (`smaps_rollup` `Pss_Anon`, `AnonHugePages` in `smaps`) after a render on the dev VM with and without `hugetlb=0`. If an arena is a huge page, make the same call at its start.

## Not in this ticket

scootbg (fixed).
