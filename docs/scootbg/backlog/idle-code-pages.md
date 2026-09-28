---
title: "Idle memory above the floor: the daemon's resident code"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Idle memory above the floor: the daemon's resident code

A loss on the release gate ([ticket 11](lightest.md)). Idle, above the
floor, `awww-daemon` holds 1.1–1.6 MiB less RSS than scootbg and 1.0–1.5
MiB less PSS (medians of 5 rounds). That holds with a color and with an
image, on 1× 1080p and on 2× 4K, on scoot and on sway alike. On 1× 1080p
with an image, the same bytes also make awww's total with the floor lower
(8.9 against 10.3 MiB). The figures are in
[the comparison](../README.md#against-the-other-daemons).

**The direction, chosen by the user: shrink the code.** No new `unsafe`
and no paging out; see the last section for the one fallback that stays
the user's decision.

## Where the bytes are

Mostly scootbg's own code: its `.text` and read-only data hold 1,356 KiB
resident against awww-daemon's 472, 884 KiB (0.86 MiB) more. Most of
the rest is `libc` code, 320 KiB more, plus a little anonymous memory
(60 KiB); awww maps `liblz4`, which scootbg does not (100 KiB). All of it is clean and file-backed
except the anonymous part. Idle with a color on headless scoot
(`/proc/PID/smaps`), awww 0.12.1 from nixpkgs, in KiB:

| Mapping | scootbg | awww-daemon |
|---|---|---|
| own `.text`, resident / whole | 1,044 / 1,260 | 364 / 372 |
| own read-only data, resident | 312 | 108 |
| `libc.so.6` code, resident | 1,336 | 1,016 |
| anonymous, all mappings | 220 | 160 |
| `[heap]` | 36 | 4 |

- **Which binaries.** This breakdown was taken with the release build
  `c2ea42c2…`, from `d8cb6dc`.
  - The page-out figures below, and every published run, used `39e2c361…`
    from the same tree (`cargo test --release` rebuilt it in between).
  - Both have the same `.text` (1,290,335 B) and `.rodata` (147,752 B).
  - The review of PR #301, on its own reproduction, read 1,088 KiB of
    scootbg's `.text` resident against awww's 312, `libc` code 1,336
    against 1,016 KiB, and `[heap]` 36 against 4 KiB: the same picture.
- **Why so much of scootbg's code is resident.** awww splits its work in
  two: `awww-daemon` only holds buffers and talks Wayland, and almost all
  of its `.text` (380,856 B) is resident (312 of 376 KiB, measured in
  review). The decoding and scaling run in the
  `awww` client, which exits. scootbg is one binary, whose `.text`
  (1,290,335 B, 1.23 MiB) holds the CLI, the client, the decoders and the
  scaler. On the color path alone, which runs no decoder, 1,044 KiB of it
  is resident. A fault maps up to 64 KiB of neighbouring page-cache pages
  (the kernel's fault-around), so code scattered through the binary
  brings in nearly all of it.
- **Paging out does not stay.** Paging the idle daemon's file pages out
  from outside (`process_madvise(MADV_PAGEOUT)`, as root) takes RSS from
  3,952 to 2,420 KiB, and PSS from 2,849 to 1,317 KiB. The 2,200 KiB of
  file RSS left are library pages other processes map too. One `scootbg
  query` faults 1,016 KiB back (3,412 KiB), and a color `set` brings it
  to 3,772 KiB.

## The levers, and what each can reach

- **A short-lived decode process** (awww's design) reaches only
  anonymous memory. After an image `set`, file-backed RSS is 3,744 KiB
  against 3,732 KiB for a color: the decoders' code adds nothing that
  stays. Anonymous memory is 468 against 220 KiB. So this lever is worth
  about 0.24 MiB with an image and nothing with a color.
- **Less code in the daemon's process** reaches the resident `.text`,
  which is the gap. A separate, smaller daemon binary is the lever at the
  limit of this: it keeps the CLI, the client half, the decoders and the
  scaler out of the long-lived process's mapping altogether. Short of
  that, the same code can be made smaller or kept together.

## The plan, in order

1. **Attribute the resident pages.** Read the idle daemon's
   `/proc/PID/pagemap` for its text mapping, and map the resident pages
   to functions with a release build with `strip = false` and the same
   layout. That says which code the loop and its start-up actually run,
   and what fault-around adds around it.
2. **Shrink what the daemon runs**, each candidate measured on this
   table before it is chosen:
   - hand-written JSON for the control protocol instead of `serde_json`,
     which cost +74 KB when it was chosen (§5 of
     [dependencies-done.md](resolved/dependencies-done.md));
   - `std::fmt` on the loop's paths;
   - `opt-level = "s"` per package (`[profile.release.package.NAME]`) for
     the cold crates: `serde_json`, the CLI and the client half. §7 of
     dependencies-done.md measured `"s"` only for every crate at once
     (−0.9% on the whole binary, 17% larger on the base, the pipeline
     about 7% slower). Per cold crate is untried, and keeps the decoders
     and the scaler at 3.
3. **A separate daemon binary**, if 1 and 2 leave a gap: `scootbg daemon`
   execs a small `scootbg-daemon` that links only the loop, and the
   decoding and scaling run in a short-lived process or thread of the
   big binary. This touches packaging (the Nix package and modules, the
   `apply-config` spawn path), so it is its own design pass.
4. **Keep the hot code together** so fault-around maps fewer windows,
   with a linker symbol-ordering file. It depends on the toolchain and is
   fragile across builds, so it needs a CI check that it still applies.

**The target** is awww's idle figures above the floor on the same
machine: RSS at or under 2.7 MiB and PSS at or under 1.0 MiB with a
color. Check it with `scripts/scootbg-bench/bench.py run --only idle`,
then `compare` against the published run.

## The fallback that is the user's call

Paging the code out when idle (`madvise(MADV_PAGEOUT)` on its own text
once the loop goes idle) would pass the row as measured. But it is a new
`unsafe` call in `scootbg-mem`, and every request after it refaults about
1 MiB. The user chose to shrink the code instead. This stays on the list
only as something the user may decide later, never as a quiet fix.
