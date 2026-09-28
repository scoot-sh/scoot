---
title: "Tests: unit, and end to end on headless scoot"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Tests: unit, and end to end on headless scoot

- Unit tests for fit-mode geometry (every mode against odd sizes, portrait
  on landscape, 1×1 sources, sizes that round at fractional scales),
  state-file round trips, and request parsing.
- End to end: a script like `scripts/smoke-test.sh` that starts
  `scoot --headless --outputs 2`, runs `scootbg daemon`, sets a color and
  an image per output, and samples pixels from `scootctl screenshot
  --output N` at known points (centre, letterbox bars, corners). The color
  half is in `crates/scootbg/tests/color.rs` since
  [solid-color-done.md](resolved/solid-color-done.md) (every pixel of each
  output, on scoot through its screenshot request and on sway through a
  wlr-screencopy client in the harness); images add the fit-mode points.
- The precedence rule end to end: every order in
  [scoot-integration-done.md](resolved/scoot-integration-done.md)'s table, plus a config
  with two per-output overrides and a changed `command` across a restart,
  where the `scootbg set` pick must survive.
- Hotplug on the headless backend if scoot can add and remove virtual
  outputs at runtime; otherwise note the gap and cover it on `--tty`.
- Fuzz the image-loading entry point with truncated and corrupt files
  (the decoders are third-party; the guard is ours).
- A `cargo fuzz` target over the whole path, decode + crop + scale +
  pack, since `pic-scale-safe` has no fuzzing upstream. It should take
  arbitrary bytes and odd target sizes: 1×1, 1×N, primes, sizes larger
  than the source, and extreme aspect ratios. Any panic is a finding,
  because under `panic = "abort"` a panic kills the daemon.
- Unit tests for `scootbg-mem`'s allocator:
  - allocations and reallocations across the 128 KiB threshold in both
    directions;
  - `alloc_zeroed` on both paths;
  - an alignment above the page size (must go to `System`);
  - a stress loop from several threads.

  For the shm buffer, a test that truncating the memfd is refused once
  the seals are set.

**From [ticket 6](resolved/images-decode-and-fit-done.md#verified-where):**
the unit tests now cut every test file (JPEG baseline and progressive,
PNG, WebP) at every length and flip random bytes in 300 copies of each,
through the real decode entry point; the scaler is called over every
shape 1–5 × 1–5 in both directions and a set of extreme aspects, every
filter. Deterministic, not coverage-guided: the `cargo fuzz` target over
decode + crop + scale + pack above is still to do, and is the place
`pic-scale-safe`'s lack of upstream fuzzing is answered.

**From [ticket 9](resolved/restore-state-done.md#review-of-pr-290):**
`query`'s per-output `draw_failed` tells a failed draw from a `clear`
(both show `shows: null`), and is covered end to end only for a restored
image that no longer decodes (`tests/restore.rs`). A failed draw on a
live `set` (a buffer too large for `wl_shm`, out of memory) is covered by
the reply's error and stderr, not yet by a `draw_failed` check; nor does
`query` say *why* a draw failed (stderr does). Worth a test, and a reason
string in `query`, if an agent ever needs to tell the causes apart.
