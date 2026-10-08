---
title: "scoot stack mapping reports 17 MB PSS at idle"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# scoot stack mapping reports 17 MB PSS at idle

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`dev/benches/benchmarks.md`). Serves **daily-drive** (if 17 MB of stack is
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

## Resolution (2026-10-05): an artifact of the benchmark's classifier, no compositor change

The 17.1 MB `[stack]` is the mapping *before* it, misattributed. The
benchmark's `smaps.sh` (`~/fx/cmpde-cerval/smaps.sh`, scratch, not in this
repo) credits each block's PSS to the *next* block's class: demonstrated
live, where an 8 MB memfd pool reads as 8 MB `anon-other` with 0 shm. The
same shift puts the highest mapping's PSS onto `[stack]`.

Raw entries from a live idle compositor (`--tty`, current main):

```
7fffcbbac000-7fffcbbd0000 rw-p 00000000 00:00 0   [stack]
Size:   144 kB
Rss:    128 kB
Pss:    128 kB
```

A 17 MB proportionally-shared main-thread stack is independently
impossible under the 8 MB `RLIMIT_STACK`. And the campaign totals agree:
fat-round-1's `shm 12,328 + stack 17,104 = 29,432 kB` is exactly
fat-round-3's `shm 29,432 kB` (round 2: 29,672) — all three fat rounds
hold the same ~29.5 MB shm (client pools: wallpaper plus foot's
double-buffered terminal), read through a one-block shift. The lean
rounds' 63.6 MB is the wallpaper pools unmapped (see
`image-retention.md`'s resolution: the image was unreadable there), not
stack saved.

Nothing to fix in the repo: per-map classification lives only in that
scratch script (this repo's `scripts/scootbg-bench/procs.py` reads
`smaps_rollup` and is unaffected). The benchmark page carries a correction
note. `RLIMIT_STACK` and the thread stacks (`[stack:TID]`, correctly
`anon-other`) were checked, not changed.
