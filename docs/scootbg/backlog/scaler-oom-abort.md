---
title: "The scaler's output allocation aborts the daemon when memory is refused"
status: "open"
area: "scootbg"
priority: "medium"
blocked: ""
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

## Decision: (b), probe then free

Chosen by the user, 2026-09-29. (c) stays the way to remove the abort
outright if (b) proves not enough in practice; (a) and (d) are not
pursued.

## The ticket

**Goal.** A scaling draw that the kernel would refuse fails like the
`center` case already does: the reply and `query` report `draw_failed`
with a `draw_error`, the daemon stays up, and the next `set` draws.

**Scope.**

- Before the scaler runs, in the one place scootbg calls it (behind the
  `MAX_SCALED_SIDE` guard in `image/scale.rs`), reserve a probe with
  `try_reserve_exact` and drop it at once. Size it as the section above
  says: the output's RGB size, plus both axes' weight tables (the
  `f32` table and its `i16` copy, `kernel_size × out_size` each, with the
  kernel width for the filter in use and the shrink/grow case), plus the
  row scratch. Compute it with checked arithmetic; an overflow is a
  refusal, not a panic.
- Refuse with a new scale error variant whose message says the draw
  needs N bytes it could not get, surfaced through the existing
  `draw_error` path. No protocol change.
- Skip the probe when nothing is scaled (same size, `center`, `tile`).
- The probe runs once per draw, not per frame or per event; it adds one
  allocation of the size the scaler is about to make anyway.

**Tests.**

- Extend `tests/draw_failed.rs`: the same lowered `RLIMIT_AS`, now with
  `--mode fill`, reports `draw_failed` and a `draw_error`, the daemon is
  still alive, and a following `set` draws. This is the case the test
  currently avoids by using `center`.
- A unit test for the probe's size against the scaler's real
  allocations for each filter, shrinking and growing, so the budget
  cannot drift below what `pic-scale-safe` takes. Measure the real
  figure (a counting allocator in the test is enough) rather than
  trusting the formula.
- The fuzz target keeps running the same path; replay its corpus.

**Docs.** `cli.md` (a scaling draw under an address-space limit now
fails cleanly), the CHANGELOG, and the limits section of
`docs/scootbg/testing.md`. State the residual plainly: under strict
overcommit another process can take the memory between the probe and
the scaler's allocation, and then the daemon still aborts.

**Done when.** The `fill` case above passes on headless scoot and sway
in CI, the full scootbg verification set is clean, and the review gate
has passed. The item then moves to `resolved/` as
`scaler-oom-abort-done.md`.
