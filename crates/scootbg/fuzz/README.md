# scootbg's fuzz target

One `cargo fuzz` target, `whole`: arbitrary bytes through the image path
the daemon runs, decode, fit, crop, scale and pack, with every mode,
filter and orientation and odd buffer sizes (1×1, 1×N, primes, larger
than the image, extreme aspect ratios, zero, and sizes past `wl_shm`'s
limit). Any panic is a finding: the daemon's release profile is
`panic = "abort"`, so a panic there kills it.

The entry point and the input layout are in
[`src/image/fuzz.rs`](../src/image/fuzz.rs). scootbg is a binary with no
library, so [`fuzz_targets/whole.rs`](fuzz_targets/whole.rs) compiles
its image modules by `#[path]`, unchanged, with the daemon's allocator.
Each input runs on a thread with the daemon's decoding stack
(`image::DECODE_STACK`, 2 MiB, shared with `daemon::worker`), not
libFuzzer's 8 MiB main thread, so an input that overflows the daemon's
stack overflows here too. The guards are the daemon's own (the pixel
budget at decode, `wl_shm`'s size limit before a buffer exists); the one
limit of the harness's own is a throughput cap, `MAX_FUZZ_PIXELS`, that
skips buffers `wl_shm` would take but are over 2^22 pixels (4
megapixels: 1080p and 1440p are in).

This crate is its own workspace, with its own `Cargo.lock`: it is never
built by `cargo build --workspace`, nextest, clippy or the flake, and
nothing here reaches the shipped binary.

## Running it

The stable toolchain the project pins, with no sanitizer (`-s none`):
`-Zsanitizer` is nightly-only, and nothing else cargo-fuzz passes needs
nightly. `cargo-fuzz` comes from the pinned nixpkgs, for this command
only; it is not in the dev shell. From the repository root:

```sh
devenv shell -- nix shell --inputs-from . nixpkgs#cargo-fuzz --command bash -c '
  cd crates/scootbg &&
  mkdir -p /tmp/scootbg-corpus &&
  cargo fuzz run -s none whole /tmp/scootbg-corpus fuzz/corpus/whole -- \
    -max_len=8192 -timeout=30 -rss_limit_mb=4096 -max_total_time=600'
```

- **Give it a scratch corpus first.** libFuzzer writes what it finds
  into the first directory; `fuzz/corpus/whole` is the committed seed
  corpus and should stay a few KB. To keep a new input worth having,
  merge into it deliberately (`cargo fuzz run -s none whole
  fuzz/corpus/whole /tmp/scootbg-corpus -- -merge=1`) and look at what
  that adds.
- **Parallel:** `-fork=N` runs N processes (add `-ignore_crashes=1` to
  keep going past a crash, and `-artifact_prefix=DIR/` to collect them).
  A forked run can still drop a reproducer (`slow-unit-*`, `crash-*`)
  in `crates/scootbg/`, its working directory; `.gitignore` keeps those
  out of a commit, so look there too.
- **Debug assertions:** `cargo fuzz run -s none -a ...` turns on
  overflow checks in every crate, the dependencies included. The daemon
  ships without them, so a panic that needs `-a` is an integer overflow
  to look at on its merits, not a crash the daemon has.
- **Disk:** the build is about 1 GB in `fuzz/target` (gitignored); pass
  `--target-dir` to put it elsewhere.
- **Slow units** are expected from images at the pixel budget (2^28
  pixels): the instrumented build is several times slower than the
  daemon, so a 264-megapixel image a few hundred bytes long (an animated
  WebP's canvas, a flat lossless WebP) takes seconds per run here. They
  cost the daemon what any image of that size costs, within the budget
  (`docs/scootbg/backlog/resolved/testing-done.md` has the numbers).

## A crash

Reproduce it with `cargo fuzz run -s none whole PATH`. Fix it in scootbg,
guarding in scootbg's own code if the panic is in a dependency (fixes to
a dependency go in a scoot-sh fork, never upstream: see
[`docs/forks.md`](../../../docs/forks.md)). Then copy the input to
`regressions/whole/` with a name that says what it was: scootbg's stable
test `image::fuzz::tests::the_corpus_and_every_past_crash_replay_cleanly`
replays that directory and the seed corpus on every `cargo test`, so CI
covers them without nightly or cargo-fuzz.

## Keeping it in step

`Cargo.lock` here started as a copy of the workspace's. The stable test
`image::fuzz::tests::the_fuzz_lockfile_matches_the_workspace` follows the
dependency edges of both locks, from `scootbg` and from `scootbg-fuzz`,
and fails when a package both reach is at a version scootbg does not
resolve to, or depends on another version of something than scootbg's
copy does (the workspace locks two `miniz_oxide` and two `rustix`, so a
check by name alone would pass the wrong one). So a dependency bump cannot
leave the fuzzer on code the daemon no longer runs. To refresh it:

```sh
cp Cargo.lock crates/scootbg/fuzz/Cargo.lock
devenv shell -- bash -c 'cd crates/scootbg/fuzz && cargo update --workspace'
```

The same test does not see a new module the image path starts to use
outside `crate::image` and `crate::color`: the fuzz build does, and fails
to compile, which is the signal to add it to `fuzz_targets/whole.rs`.

## The seed corpus

`corpus/whole/` holds 21 inputs, 6.4 KB in all, each the 19-byte header
([`src/image/fuzz.rs`](../src/image/fuzz.rs) documents it) and a small
file: the three JPEG fixtures in `tests/fixtures/`; JPEGs at 4:2:0 and
4:2:2, progressive, with a big-endian and a little-endian EXIF
orientation, CMYK, and one claiming 16384×16384; PNGs at 16-bit RGBA,
16-bit grey interlaced, palette with transparency, grey + alpha, 1×1,
and one claiming 16384×16384; lossless, lossy, lossy-with-alpha and
animated WebPs. They were made with ImageMagick 7.1.2 (the EXIF blocks
spliced in by hand), and span every mode, filter and orientation, and
sizes at and past `wl_shm`'s limit.
