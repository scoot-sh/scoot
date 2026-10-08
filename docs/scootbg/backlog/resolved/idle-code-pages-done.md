---
title: "Idle memory above the floor: the daemon's resident code"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Idle memory above the floor: the daemon's resident code

The release gate's one failing class ([ticket 11](lightest.md)). Idle,
above the floor, `awww-daemon` holds 1.1–1.5 MiB less RSS than scootbg
and 1.0–1.4 MiB less PSS (medians of 5 rounds, re-run 2026-09-28; 1.1–1.6
and 1.0–1.5 MiB in the first run, 2026-09-27). That holds with a color
and with an image, on 1× 1080p and on 2× 4K, on scoot and on sway alike:
9 gated rows on each compositor. On 1× 1080p with an image, the same
bytes also make awww's total with the floor lower (8.84 against 10.25
MiB on scoot). The figures are in
[the comparison](../README.md#against-the-other-daemons).

**Waived for v1 (user, 2026-09-28).** The gate does not pass: the cheap
levers are done and the gap remains (1.0–1.5 MiB). The user waived this
class for v1 rather than split the binary now, so this item is post-v1.
The separate daemon binary below is the lever to reach for then. The
bytes are mostly clean program code the kernel can reclaim; the rest is
anonymous memory (below).

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
  about 0.24 MiB with an image and nothing with a color. (The attribution
  below finds that anonymous excess is the decode thread's glibc malloc
  arena, and that the decoders' code is resident with a color too, which
  only a binary without it avoids.)
- **Less code in the daemon's process** reaches the resident `.text`,
  which is the gap. A separate, smaller daemon binary is the lever at the
  limit of this: it keeps the CLI, the client half, the decoders and the
  scaler out of the long-lived process's mapping altogether. Short of
  that, the same code can be made smaller or kept together.

## Attribution (2026-09-28)

**Method.** A release build of `286cb1d` with symbols
(`CARGO_PROFILE_RELEASE_STRIP=false`, in its own `CARGO_TARGET_DIR`): the
same `.text` (1,290,335 B) and layout as the published `39e2c361…`. The
daemon ran on headless scoot (`d84d06d0…`), 1× 1920×1080, and was set to
`#1e1e2e` or to the 6000×4000 JPEG, then left idle. Two readings:

- **Resident**: `/proc/PID/pagemap` for the binary's mappings, each
  present page mapped to the functions it holds (`nm -S`), by crate.
- **Executed**: the same run under callgrind (valgrind 3.27.1 from the
  flake's nixpkgs), start-up, the `set`, the idle loop, a `query` and a
  color `set`, every function with a cost counted whole.

Function bytes by crate group, KiB (the binary holds 1,248 KiB of
functions):

| Group | In the binary | Color: resident | Color: executed | Image: resident | Image: executed |
|---|---|---|---|---|---|
| decoders and scaler (zune-jpeg, image-webp, png, fdeflate, pic-scale-safe, …) | 398 | 277 | 0 | 336 | 82 |
| core, std, alloc | 287 | 261 | 45 | 287 | 53 |
| scootbg's own | 269 | 269 | 154 | 269 | 194 |
| std's backtrace and symbolization (gimli, addr2line, rustc-demangle, miniz_oxide) | 122 | 113 | 0 | 122 | 0 |
| Wayland (wayland-client, -backend, -protocols, smallvec) | 102 | 102 | 70 | 102 | 73 |
| serde, serde_json | 30 | 30 | 6 | 30 | 6 |
| other | 40 | 18 | 2 | 39 | 4 |
| **total** | **1,248** | **1,070** | **277** | **1,184** | **411** |

- **What runs is small; what is resident is nearly all of it.** With a
  color, start-up, the loop, a `query` and a `set` run 277 KiB of
  functions, while 1,048–1,080 KiB of `.text` is resident (262–270 of
  316 pages, over runs). With an image, 411 KiB run and 1,200 KiB (300
  pages) are resident. A `query` adds 64 KiB, a color `set` after it
  nothing.
- **Fault-around is the rest.** The pages holding executed functions,
  widened to the kernel's 64 KiB fault-around windows, cover 1,144 KiB of
  `.text`: the executed code touches nearly every window of the binary,
  because the linker interleaves the crates. 277 KiB of decoder and scaler
  code is resident with a color, never run.
- **The backtrace machinery is resident and never runs**: 113 KiB with a
  color, 122 KiB with an image, 0 KiB executed. It is not the panic hook
  of ticket 2 (`daemon::crash`) that links it: std's `default_hook`, which
  `rust_panic_with_hook` calls whenever no hook is set, is linked into
  every binary that can panic, `panic = "abort"` or not. Checked with a
  trivial binary under this workspace's release profile: 233 KiB of
  functions, 174 KiB of them backtrace code, and the same with a custom
  hook that never calls the default one (`default_hook` still linked).
- **libc: 320 KiB more than awww's, three causes.** Mapped per call site
  through callgrind's list of libc functions run, against awww-daemon's:
  - std's start-up guard for the main thread, `pthread_getattr_np`, which
    reads `/proc/self/maps` with `fopen`, `getline` and `sscanf` (two
    64 KiB windows; awww-daemon runs none of these, and it cannot be
    skipped from safe Rust);
  - the saver thread's exit (`__libc_thread_freeres`, the resolver's and
    RPC's per-thread cleanup: two windows, 128 KiB);
  - `std::process::id` (libc's `getpid`, 64 KiB window) for the state
    file's temporary name.
  With saving turned off, libc's resident code is 1,144 KiB against
  1,336 (measured with the prototype below, whose calls into libc are the
  daemon's).
- **With an image, anonymous memory is 248 KiB more** (468 against 220):
  a 436 KiB mapping holding 232 KiB resident, the glibc malloc arena the
  decode thread's small allocations made, which glibc keeps for the next
  thread. (Blocks of 128 KiB and more are `scootbg-mem`'s own mappings and
  go back to the kernel.) The decode thread's cached stack is 24 KiB.

## Levers tried (2026-09-28)

Each measured before it was kept. Residency with `pagemap` as above;
timings with the `image::bench` stage benchmark on the 6000×4000 JPEG,
`fill` at 3840×2160.

| Lever | Before | After | Kept? |
|---|---|---|---|
| Remove std's backtrace code, stable toolchain | 122 KiB linked, 113–122 resident | not possible on stable | no |
| `opt-level = "s"` for `serde_json` | `.text` 1,290,335 B | 1,287,663 B (−2.6 KiB) | no |
| `opt-level = "s"` for wayland-client, -backend, -protocols, -protocols-wlr | 1,290,335 B | 1,345,567 B (+54 KiB) | no |
| `opt-level = "s"` for `scootbg` itself (the root crate) | 1,290,335 B; `render()` 450–517 ms | 969,919 B, 116 KiB less resident with a color, 252 KiB with an image; `render()` 550–586 ms | no |
| The state file's temporary name through `rustix::process::getpid` | libc code 1,336 KiB resident | 1,272 KiB (three runs each, every time) | **yes** |

- **The backtrace code.** On stable there is no way to leave std's
  `default_hook`, and with it the symbolizer, out of a binary that can
  panic (above). The way that exists is nightly-only: `-Zbuild-std` with
  std's `panic_immediate_abort` feature. Not tried: the toolchain stays
  stable.
- **Per-package `opt-level`.** The crates cold for the daemon that are not
  also hot for its images are few: `serde_json` and the Wayland crates,
  and scoot uses every one of them too (its IPC, its client and server
  sides), so a workspace override changes scoot's release build as well.
  They did not help anyway: `"s"` saves 2.6 KiB on `serde_json` and costs
  54 KiB on the Wayland crates, since fat LTO re-optimizes everything at
  the root crate's level. The root crate's own level is what counts:
  `"s"` for `scootbg` takes 313 KiB off `.text`, but it is also the image
  pipeline's crate, and `render()` then runs about 20% slower (three
  runs each, five renders a run: 460, 450, 517 ms at 3 against 582, 550,
  586 at `"s"`). That would lose the JPEG start-up row to wbg, a tie today
  at 4.7%. `"z"` for the root: 25% slower. Hence no.
- **`getpid`.** One call site; the same value. It takes one 64 KiB libc
  window off every idle row that has saved anything, that is, all of them.
- **Measured, not kept, and why:**
  - *A saver thread that stays* (parked on its condition variable instead
    of exiting) takes libc's two thread-exit windows off: libc code 1,272
    → 1,144 KiB with a color. But an image's decode thread exits the same
    way, so the image rows keep them, and it changes the daemon's idle
    thread count from one to two. Worth it only together with the split
    below, where no decode thread is left.
  - *Lazy binding* would keep libm's pages (256 + 88 KiB, the scaler's
    `sinf`, resolved at start-up under `-z now`) out of the color rows,
    but gives up full RELRO, a hardening default, and needs `RUSTFLAGS`.
    Not tried.

## Where it stands (2026-09-28)

The gate's idle rows, 5 rounds, re-run with `35f3a13`
([`bench/2026-09-28-idle-scoot/`](../bench/2026-09-28-idle-scoot/table.md)
and [`bench/2026-09-28-idle-sway/`](../bench/2026-09-28-idle-sway/table.md)).
Medians and ranges above the floor, MiB, on headless scoot (the raw KiB
over 1,024, rounded to two places; each gap from the unrounded medians,
so it can differ by 0.01 from the difference of the columns: 4,500 −
2,948 KiB is 1.52 MiB):

| Row | scootbg before (2026-09-27) | scootbg now | awww now | Gap now |
|---|---|---|---|---|
| RSS, 1× 1080p, color | 3.85 [3.81–3.89] | 3.77 [3.75–3.86] | 2.71 [2.66–2.74] | 1.06 |
| PSS, 1× 1080p, color | 2.07 [2.01–2.09] | 2.02 [2.02–2.08] | 1.04 [0.99–1.06] | 0.98 |
| RSS, 2× 4K, color | 3.87 [3.80–3.93] | 3.79 [3.74–3.82] | 2.69 [2.65–2.74] | 1.10 |
| PSS, 2× 4K, color | 2.07 [2.02–2.10] | 2.05 [2.00–2.06] | 1.04 [1.00–1.07] | 1.01 |
| RSS, 1× 1080p, image | 4.38 [4.31–4.41] | 4.29 [4.23–4.32] | 2.84 [2.82–2.92] | 1.45 |
| PSS, 1× 1080p, image | 2.35 [2.28–2.39] | 2.34 [2.29–2.35] | 0.93 [0.92–1.01] | 1.42 |
| RSS, 2× 4K, image | 4.49 [4.44–4.53] | 4.39 [4.34–4.46] | 2.88 [2.86–2.94] | 1.52 |
| PSS, 2× 4K, image | 2.47 [2.39–2.50] | 2.40 [2.38–2.48] | 0.99 [0.96–1.00] | 1.41 |

On headless sway, the same rows: color RSS 3.77 and 3.79 against awww's
2.67 and 2.69 (gap 1.10, 1.09), PSS 2.00 and 2.03 against 1.02 and 1.04
(0.98, 0.99); image RSS 4.30 and 4.37 against 2.89 and 2.90 (1.41, 1.47),
PSS 2.31 and 2.39 against 0.96 and 0.95 (1.35, 1.44). Before
(2026-09-27): color RSS 3.88 and 3.87, PSS 2.06 and 2.08; image RSS 4.38
and 4.45, PSS 2.34 and 2.39.

**Not closed.** The same 9 losses on each compositor. `compare` against
the 2026-09-27 run finds no regression and no change beyond the margin:
the kept lever is 64 KiB, inside the noise. What is left is the whole
gap, and the attribution says where it is: the daemon runs 277 KiB of a
binary holding 1,248 KiB of functions, and fault-around makes most of the
rest resident whatever the daemon runs.

## Later option: a separate daemon binary

Post-v1: the user waived the gap for v1 rather than take this on now
(2026-09-28). Not started (the scope of 2026-09-28 was the attribution
and the cheap levers).

**What was measured.** A prototype, never committed: the daemon's code in
its own crate and binary, the decoders stubbed out (so image rows could
not be run), plus the `getpid` lever and a saver thread that stays. Run
with the harness's own idle scenario (`scenarios.idle`, 3 rounds, 60 s
idle, a 5 s wakeup window, scootbg and awww in one batch) on headless
scoot, medians above the floor, MiB:

| Row | Current binary | Prototype | awww, in the prototype's batch |
|---|---|---|---|
| RSS, 1× 1080p, color | 3.85 [3.84–3.86] | 2.59 [2.58–2.60] | 2.71 [2.68–2.71] |
| PSS, 1× 1080p, color | 2.24 [2.23–2.25] | 1.27 [1.26–1.28] | 1.25 [1.24–1.25] |
| RSS, 2× 4K, color | 3.83 [3.82–3.85] | 2.58 [2.57–2.59] | 2.67 [2.66–2.72] |
| PSS, 2× 4K, color | 2.22 [2.19–2.24] | 1.27 [1.25–1.28] | 1.25 [1.19–1.26] |

("Current binary" is the published `39e2c361…`, run the same way.) That
is a tie on PSS and a tie or better on RSS, for colors. (PSS reads
higher than in the full batch, which shares the libraries among more
processes.) The same prototype without the two libc levers: 2.82 RSS and
1.46 PSS on 1080p, a PSS loss.

**What it takes**, as the prototype found:

- **Its own package.** The daemon's `.text` is 466 KiB only when its root
  crate *and* its code are built at `opt-level = "z"`: fat LTO optimizes
  at the root crate's level (root at 3 with the code at `"z"`: about
  540 KiB; everything at 3: 658 KiB). The decoders cannot share that
  package: the image pipeline, even built at 3 in a crate of its own, runs
  25% slower under a `"z"` root (440–452 against 546–601 ms, three runs of
  five).
  So `scootbg-daemon` is a second package, and `scootbg`'s integration
  tests need it built (`-p scootbg-daemon`, or `--workspace`), which
  changes the documented test commands, CI and the Nix package.
- **The decoders in a worker process**: the daemon runs `scootbg` per
  image job, which sends the drawn buffers back as sealed memfds over a
  socket (`SCM_RIGHTS`, safe `rustix`), and the daemon hands them to the
  compositor without mapping them (no new `unsafe`). The spawn costs the
  daemon 8 KiB of code and one 64 KiB libc window; with an image the
  margin would be thin, not measured.
- **User-visible**: the daemon's process is `scootbg-daemon` (`pkill
  scootbg` still matches it; `pkill -x scootbg` would not); `scootbg
  daemon` execs it and `apply-config` starts it, found beside `scootbg`.
- **Beyond that**: `serde_json` out of the daemon (hand-written JSON, a
  few tens of KiB) would widen the margin; the Wayland crates stay at 3,
  since scoot shares them.

## Re-measurement (2026-10-08): measured and declined

Re-measured on the current tree instead of building the split, and
declined the split: the prize is about 1 MiB of kernel-reclaimable file
pages, and every lever that reaches it is machinery out of proportion to
that saving.

**Method.** Release build of `3634fa4b` (`.text` 1,204,292 B, `.rodata`
158,120 B, stripped file 1,643,296 B) with its own `CARGO_TARGET_DIR`,
on the Asahi M2 under `nix develop --offline`, against headless scoot
from the same tree at 1× 1920×1080: `scootbg daemon`, one `set` of
`#1e1e2e` or of `site/public/og-cat.jpg`, then 30 s idle. RSS/PSS from
`/proc/PID/smaps_rollup`, the mapping split from `/proc/PID/smaps`
(`r-xp`/`r--p` of the `scootbg` mapping, `r-xp` of `libc.so.6`,
`[heap]`). Own `XDG_RUNTIME_DIR` per run (mode 700), scratch `HOME`,
nothing in `/run/user/1000`. This box runs 16 KiB pages, so one
fault-around window is four of them; the Sept attribution ran on 4 KiB
pages, and the two are compared as KiB resident, not as page counts.

**Numbers** (load average beside each; the box is shared, load 2–10):

| Run | Load (1 min) | RSS, KiB | PSS, KiB | Anonymous, KiB |
|---|---|---|---|---|
| color 1 | 2.86 | 3,568 | 2,269 | 368 |
| color 2 | 2.15 | 3,600 | 2,134 | 384 |
| color 3 | 2.31 | 3,584 | 1,837 | 368 |
| image 1 (og-cat, fill) | 6.26 | 12,176 | 6,334 | 656 |
| image 2 (og-cat, fill) | 6.36 | 12,176 | 6,228 | 656 |

Mapping split at idle (extra runs, same setup; color at load 10.01,
image at 7.85), KiB resident:

| Mapping | Color | Image |
|---|---|---|
| own executable (`r-xp`) | 1,344 | 1,408 |
| own read-only (`r--p`) | 64 | 64 |
| `libc.so.6` executable | 1,472 | 1,472 |
| `[heap]` | 48 | 48 |

Lifetime `voluntary_ctxt_switches` 8–10 after start, one `set`, one
`query` and the 30 s idle: nothing wakes the idle daemon. The picture
is the ticket's: nearly all of the daemon's mapped code is resident
(1,344 KiB of a 1,552 KiB executable mapping with a color), the image
adds one 64 KiB window of decode path, and RSS minus anonymous
(3,216 KiB with a color) is file-backed pages the kernel drops under
pressure and reads back on demand. The unreclaimable part is the
anonymous 368 KiB (656 with an image).

**Bounds on what is left.** No non-split lever reaches more than a
fraction of a MiB: the Sept attribution (same `.text` order, 1.29
against 1.20 MiB now) found 277 KiB of resident decoder/scaler code
that never runs on the color path plus 113 KiB of never-run backtrace
code, 390 KiB together, and backtrace code cannot leave a stable
binary; `serde_json` out is a few tens of KiB for hand-rolled protocol
parsing; the parked saver thread is 128 KiB of `libc` on the color
rows only, for a second idle thread. The split's own prototype reached
1.26 MiB RSS / 0.97 MiB PSS on the color rows — that is the ceiling,
and it costs a second package, a decode worker with a sealed-memfd
socket protocol, spawn and version-skew handling, and packaging, CI
and Nix changes, in a daemon held to the crash-is-data-loss bar. The
maintainer cancelled a 1,700-line linker scheme for under 0.5 MiB on
the same grounds; this is the same trade at about twice the bytes and
several times the lines. So: declined, with the numbers above. The
`lightest.md` gate keeps its v1 waiver for exactly this class, and the
page-out fallback below stays the user's call, untouched.

No code changed for this entry, so there is no before/after test to
revert-run-restore; the verification is the measurement method above
plus the standard set on the docs-only PR.

## The fallback that is the user's call

Paging the code out when idle (`madvise(MADV_PAGEOUT)` on its own text
once the loop goes idle) would pass the row as measured. But it is a new
`unsafe` call in `scootbg-mem`, and every request after it refaults about
1 MiB. The user chose to shrink the code instead. This stays on the list
only as something the user may decide later, never as a quiet fix.
