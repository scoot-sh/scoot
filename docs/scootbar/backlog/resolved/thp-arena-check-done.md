---
title: "scootbar links the same allocator: check it for the glibc 2.44 huge-page arena"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
---

# scootbar links the same allocator: check it for the glibc 2.44 huge-page arena

Filed 2026-10-28. Serves **daily-drive**: scootbar is a resident daemon with a small-footprint goal.

## The gap

scootbg's settled daemon held 2.5 MB of anonymous memory on the aarch64 dev VM (glibc 2.44, kernel THP mode `madvise`): the decode thread's malloc arena was one resident 2 MiB huge page. `GLIBC_TUNABLES=glibc.malloc.hugetlb=0` gave 0.6 MB. scootbg now sets `PR_SET_THP_DISABLE` in `daemon::run`. scootbar shares the `scootbg-mem` allocator and any worker-thread arena, and was not measured.

## What to do

Measure scootbar's anonymous PSS (`smaps_rollup` `Pss_Anon`, `AnonHugePages` in `smaps`) after a render on the dev VM with and without `hugetlb=0`. If an arena is a huge page, make the same call at its start.

## Not in this ticket

scootbg (fixed).

## Resolution

Measured 2026-11-05 on the aarch64 dev VM (glibc 2.44, kernel 6.18.50, THP `madvise`), release build of `45eedc95d`, headless scoot, a workspace switch, 10 s settle:

- The default bar (workspaces, clock, window title): one thread, `Pss_Anon` 500-508 kB, `AnonHugePages` 0 in 9 of 9 runs, with or without `GLIBC_TUNABLES=glibc.malloc.hugetlb=0`.
- All thirteen modules placed: `[heap]` (the main heap, not a worker-thread arena; the bar had one thread) held a resident 2048 kB huge page in 4 of 8 default runs (`Pss_Anon` 2852-3308 kB) and in 0 of 8 runs with `hugetlb=0` (1308 kB).

So the arena was a huge page, on the main heap and only with many modules. `main` now sets `PR_SET_THP_DISABLE` before it parses the command line (set later, in `daemon::run`, 2 of 10 runs still held the page): 0 of 20 runs held one after, `Pss_Anon` 1304-1308 kB. The setting is inherited across `exec`, which scootbg never meets: scootbar starts click commands, menus and `exec` module commands, so those get huge pages back (`src/thp.rs`).
