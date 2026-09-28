---
title: "Idle memory above the floor: the daemon's resident code"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Idle memory above the floor: the daemon's resident code

A loss on the release gate ([ticket 11](lightest.md)). Idle, above the
floor, `awww-daemon` holds 1.1–1.6 MiB less RSS than scootbg and
1.0–1.5 MiB less PSS (medians of 5 rounds), with a color and with an image, on 1× 1080p
and on 2× 4K, on scoot and on sway alike. On 1× 1080p with an image, the
same bytes also make awww's total with the floor lower. The figures are
in [the comparison](../README.md#against-the-other-daemons).

## Where the bytes are

The gap is clean, file-backed code, not heap. Per mapping, idle with a
color on headless scoot (`/proc/PID/smaps`, release build `c2ea42c2…` at
`d8cb6dc`; awww 0.12.1 from nixpkgs), in kB:

| Mapping | scootbg | awww-daemon |
|---|---|---|
| own `.text`, resident / whole | 1,044 / 1,260 | 364 / 372 |
| own read-only data, resident | 312 | 108 |
| `libc.so.6` code, resident | 1,336 | 1,016 |
| anonymous, all mappings | 220 | 160 |
| `[heap]` | 36 | 4 |

- awww splits its work in two: `awww-daemon` only holds buffers and
  talks Wayland, and its whole `.text` (380,856 B) is resident. The
  decoding and scaling run in the `awww` client, which exits.
- scootbg is one binary: `.text` is 1,290,335 B, holding the CLI, the
  client, the decoders and the scaler. On the color path alone, which
  runs no decoder, 1,044 kB of it is resident. A fault maps up to 64 KiB
  of neighbouring page-cache pages (the kernel's fault-around), so code
  scattered through the binary brings in nearly all of it.
- **Not the decoders.** After an image `set`, file-backed RSS is 3,744
  kB, against 3,732 kB for a color; only anonymous memory grows (468
  against 220 kB). Moving decoding into a short-lived process, awww's
  design, would save about 0.25 MB, not the gap.
- **Reclaimable, but only until the next request.** Paging the idle
  daemon's file pages out from outside (`process_madvise(MADV_PAGEOUT)`,
  as root) takes RSS from 3,952 to 2,420 kB, and PSS from 2,849 to 1,317
  kB. The 2,200 kB of file RSS left are library pages other processes
  map too. One `scootbg query` faults 1.0 MB of it back (3,412 kB), and a
  color `set` brings it to 3,772 kB.

## What would fix it, in order of what to try

1. **Attribute the resident pages.** Read the idle daemon's
   `/proc/PID/pagemap` for the text mapping, and map the resident pages to
   functions with a release build with `strip = false` and the same
   layout. That says which code the daemon's loop and its start-up
   actually run, and what fault-around adds around it.
2. **Run less code in the long-lived process.** Candidates, each to be
   measured before it is chosen:
   - hand-written JSON for the control protocol instead of `serde_json`
     (+74 KB when chosen, §5 of
     [dependencies-done.md](resolved/dependencies-done.md));
   - `std::fmt` on the loop's paths;
   - the CLI and the client half, which only a separate daemon binary
     would keep out.
3. **Lay out the daemon's hot code together**, so fault-around maps fewer
   windows: a linker symbol-ordering file. It depends on the toolchain and
   is fragile across builds, so it needs a check in CI that it still
   applies.
4. **Page the code out when idle** (`madvise(MADV_PAGEOUT)` on its own
   text once the loop goes idle). This is a new `unsafe` call in
   `scootbg-mem`, and every request after it refaults about 1 MB. It is
   a policy choice for the user, not a fix to make quietly.

The target is awww's idle figures above the floor on the same machine:
RSS at or under 2.7 MB and PSS at or under 1.0 MB with a color. Re-run
`scripts/scootbg-bench/bench.py run --only idle` to check.
