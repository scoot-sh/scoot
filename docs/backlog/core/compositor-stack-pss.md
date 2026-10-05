---
title: "scoot stack mapping reports 17 MB PSS at idle"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# scoot stack mapping reports 17 MB PSS at idle

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`docs/benchmarks.md`). Serves **daily-drive** (if 17 MB of stack is
really touched, that is a quarter of the compositor's footprint; if it
is a measurement artifact, the benchmark's mapping story is wrong).

## The gap

`smaps` PSS-by-mapping-class on the idle scoot compositor (origin/main
`5f96802`, `--tty`, moonrise, one foot) reports, in all 3 rounds:

- anon-other 8.2 MB, file-so 32.2 MB, file-other 5.6 MB, shm 12.3 MB,
  heap 32 kB — and **stack 17.1 MB PSS in 1 map**
  (`ev/scoot-r1-smaps-scoot-*.txt` and siblings; total 75.5 MB agrees
  with `smaps_rollup` 75.6 MB, so the classes add up).

A 17 MB *proportionally-shared* stack is suspicious: the main thread's
stack cannot exceed its rlimit and share that much unless deeply
touched, and no other thread's stack is named `[stack]`. Either scoot
touches an enormous amount of stack at idle (deep call chains? a huge
stack buffer somewhere in startup that never returns?) or the
classifier lumps something else under that name.

For comparison the whole heap is 32 kB: scoot's own allocation story
is exemplary, which is what makes the stack row worth checking rather
than assuming.

## What to do

Read the raw `[stack]` (and `[stack:TID]`) entries (Size/Rss/Pss/Swap)
on a live idle session, check `RLIMIT_STACK` and the actual thread
stacks (pmap/`/proc/PID/task/*/maps`), and find what the 17 MB is.
If it is real touched stack, find the deep path and shrink it
(expected saving: up to ~17 MB PSS). If it is an artifact (guard pages
counted? a mislabeled mapping?), fix the benchmark's classifier and
note it. Either way, record the raw entries in the ticket.

## Not in this ticket

The shm 12.3 MB (client `wl_shm` buffers, expected — see the existing
VM benchmark); file-so 32 MB (Mesa/LLVM file-backed, shared).
