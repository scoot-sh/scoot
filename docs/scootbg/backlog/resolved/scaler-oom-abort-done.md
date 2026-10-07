---
title: "The scaler's output allocation aborts the daemon when memory is refused"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# The scaler's output allocation aborts the daemon when memory is refused

Found while writing ticket 12's out-of-memory test
([testing-done.md](resolved/testing-done.md#found-along-the-way)).

## The abort

`pic-scale-safe` 0.1.12's `resize_rgb8` (`src/resizer.rs`, through
`resize_fixed_point` in `src/resize_fixed_point.rs`) allocates the scaled
image itself, with an infallible `vec![T::default(); dst_stride *
height]` (the `Nearest` path's `store` likewise), and returns it. It has
no public entry point that takes a destination slice. When the allocator
refuses that block, Rust's `handle_alloc_error` aborts the process.

So any draw that scales (`fill`, `fit`, `stretch`, and an image of
another size than the output) aborts the daemon when the kernel refuses
the output's size in memory:

- under an address-space limit: `RLIMIT_AS`, systemd's `LimitAS=`,
  `ulimit -v`;
- under strict overcommit, `vm.overcommit_memory=2`.

Measured by hand on a 3840×2160 headless scoot output, the daemon's soft
`RLIMIT_AS` lowered to its idle size plus 16 MiB: a 1×1 PNG `set` with
`--mode fill` ends the daemon with `memory allocation of 24883200 bytes
failed`, exit 134 (SIGABRT). The same `set` with `--mode center`, which
scales nothing, fails gracefully instead: the `wl_shm` buffer's `mmap` is
fallible, and `query` reports `draw_failed` with
`draw_error: "shared memory: Cannot allocate memory (os error 12)"`
(`crates/scootbg/tests/draw_failed.rs`).

### The weight tables, too

The output is not the scaler's only large allocation. For each scaled
axis `generate_weights` (`src/compute_weights.rs`) allocates
`kernel_size × out_size` `f32`s, the kernel spanning `2 × support × in /
out` source pixels when shrinking (`support` 3 for Lanczos3, 2 for
Catmull-Rom) and `2 × support` when growing, and
`numerical_approximation_i16` copies them as `i16`: about 36 bytes per
pixel of the axis's longer side for Lanczos3, committed as they are
written, all infallible `Vec`s. Before ticket 12's review that was
unbounded (a 16.7-million-pixel row cost about 600 MB of weights for 50
MB of RGB, and past 2^24 the weights panicked outright); since
`scale::MAX_SCALED_SIDE` (65536) refuses longer sides, they are at most
about 2.4 MB an axis. They matter to the options:

- **(c), the fork, does not make them fallible.** A destination-slice
  entry point moves the output into scootbg's fallible buffer; the
  weights stay the scaler's own `Vec`s. At the bound's 2.4 MB an axis
  that is a small, bounded remainder, but it is not zero.
- **(b)'s probe must budget them**, output + both axes' weights
  (+ the row scratch), not the output alone.

## How much it matters

- **Rare on a default desktop.** Overcommit is heuristic (mode 0), so a
  24 MB request is granted even when memory is tight, and a cgroup
  `memory.max` OOM-kills a process (or reclaims) rather than returning
  `ENOMEM` to it. The abort needs one of the settings above.
- **Only the wallpaper daemon dies.** scoot, and every other client, are
  unaffected; the output shows the compositor's background until
  scootbg is started again (scoot's `[wallpaper]` section runs it again
  on the next reload), which restores the last wallpaper.
- It is the same class as the decoders' own working memory
  ([images-decode-and-fit-done.md](resolved/images-decode-and-fit-done.md),
  "Infallible allocations outside our buffer"); the scaler's output is
  the largest of those in an ordinary `set`, the size of the output in
  RGB.

## Options

- **(a) Accept it and document it.** Say in `cli.md` that a daemon under
  an address-space limit or strict overcommit can be ended by a large
  draw. No code.
- **(b) Probe, then free.** Before scaling, `try_reserve_exact` the
  output's size plus the scaler's transient scratch, and free it again;
  refuse the draw (a `draw_error`) if that fails. Cheap. Nearly sound
  under `RLIMIT_AS`, since the daemon runs one job at a time and nothing
  else in it allocates much meanwhile; racy under strict overcommit,
  where another process can take the commit charge between the probe and
  the scaler's allocation. A heuristic, not a guarantee.
- **(c) A scoot-sh fork of `pic-scale-safe`** with a destination-slice
  entry point (`resize_rgb8_into(src, src_size, dst: &mut [u8],
  dst_size, filter)`). Its internal convolution functions already take
  `&mut destination` (`convolve_trampoline_fixed_point`,
  `resize_nearest`), so the change is small; scootbg would allocate the
  destination with its fallible `scootbg_mem::zeroed_bytes`, as the
  decoders' buffers are. Per the project's fork rule (root `CLAUDE.md`):
  one upstream commit plus the fewest carried commits, pinned by rev,
  listed in `docs/forks.md`, and no upstream PR from here. The scaler's
  own row scratch stays infallible, but it is a few rows, not an image.
- **(d) Switch scalers** to one with a fallible or caller-provided
  destination. The choice of `pic-scale-safe` was measured
  ([dependencies-done.md](resolved/dependencies-done.md) §3b: no
  `unsafe`, 174 ms and the output only in memory for 6000×4000 → 4K), so
  a replacement would have to be measured against the same bar.

Waiting on the user's choice; (c) is the one that removes the abort
rather than narrowing or documenting it.

## Resolution (2026-10-07): (b) probe-then-refuse, plus (a) docs

Decided in chat: option (b), with option (a)'s docs. No fork (c), no
scaler switch (d).

`scale::scale` (`crates/scootbg/src/image/scale.rs`) now probes before
every scaling draw (`fill`, `fit`, `stretch`, any image whose size
differs from the output, and the `Nearest` path): it `try_reserve_exact`s
`probe_budget` — the output, each scaled axis's weight tables, and the
transient row scratch when both axes scale — frees it, and only then calls
the scaler. A refused
probe is `ScaleError::NoMemory` (`out of memory: cannot allocate {bytes}
bytes for scaling`), which travels the existing `draw_failed` /
`draw_error` path (the `center` mode's shared-memory failure shape), never
an abort.

The budget is re-derived from the pinned `pic-scale-safe` 0.1.12 source
for every filter scootbg can select, not taken from this ticket's "about
36 bytes" summary: per scaled axis `kernel × out × 6 + out × 32 + kernel
× 20` (the `f32` table plus its `i16` copy, both bounds copies, the
conversion scratch/order and the generation temp), with `kernel =
round(base × max(in / out, 1))` (bases 6/4/2 for Lanczos3/Catmull-Rom/
Bilinear) plus one guard tap, plus the trampoline row scratch
(`src_width × 3 × min(4, dst_height)`) when both axes scale, plus 64 KiB
of page-rounding cover. `Nearest` budgets the output alone. Worked
numbers: 1×1 → 3840×2160 is 25,393,028 bytes for Lanczos3, 24,948,736
for `Nearest` (unit-pinned in `scale::tests`). The code comments and the
site docs say honestly what the probe is: nearly sound under `RLIMIT_AS`,
racy under strict overcommit, conservative about fragmentation.

Evidence: `crates/scootbg/tests/scaler_probe.rs` — the ticket's
reproduction as `scaling_draws_refuse_under_a_tight_limit` (idle + 16
MiB, 1×1 PNG, `fill`, all four filters: daemon lives, `draw_failed` with
the probe's `draw_error`, the next `set` draws), failing before (the
daemon dies mid-draw: `the daemon closed the connection without
replying`) and passing after; plus
`a_limit_just_above_the_full_draw_budget_still_draws` (budget + 33 MB
buffer + 8 MiB headroom draws with every filter). Strict overcommit is
not tested (`vm.overcommit_memory=2` is a system setting, untouched) and
is documented as the remaining race, alongside the decoders' still
infallible working memory. Docs in `site/src/content/docs/scootbg/`
(`cli.md`, `images.md`, a troubleshooting symptom box with the
diagnosing commands and the `LimitAS=` / `ulimit -v` / strict-overcommit
causes). Benchmarks, 6000×4000 → 4K `fill`, release, 5 runs on the Asahi
M2: before — scale 112.4–112.5 ms, peak 87,648 kB, heap 752 kB;
after — scale 112.5–112.6 ms, peak 87,648–87,776 kB, heap 752 kB (the
probe lives inside the timed scale stage: +0.1 ms at most, noise);
release binary 1,577,760 B both before and after, `.text` 1,470,754 →
1,471,850 B (+1,096 B, +0.07%). No ratchet row regresses beyond the
noise (see the PR).
