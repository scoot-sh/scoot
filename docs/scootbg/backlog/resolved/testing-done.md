---
title: "Tests: unit, and end to end on headless scoot"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Tests: unit, and end to end on headless scoot — RESOLVED

- Unit tests for fit-mode geometry (every mode against odd sizes, portrait
  on landscape, 1×1 sources, sizes that round at fractional scales),
  state-file round trips, and request parsing.
- End to end: a script like `scripts/smoke-test.sh` that starts
  `scoot --headless --outputs 2`, runs `scootbg daemon`, sets a color and
  an image per output, and samples pixels from `scootctl screenshot
  --output N` at known points (centre, letterbox bars, corners). The color
  half is in `crates/scootbg/tests/color.rs` since
  [solid-color-done.md](solid-color-done.md) (every pixel of each
  output, on scoot through its screenshot request and on sway through a
  wlr-screencopy client in the harness); images add the fit-mode points.
- The precedence rule end to end: every order in
  [scoot-integration-done.md](scoot-integration-done.md)'s table, plus a config
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

**From [ticket 6](images-decode-and-fit-done.md#verified-where):**
the unit tests now cut every test file (JPEG baseline and progressive,
PNG, WebP) at every length and flip random bytes in 300 copies of each,
through the real decode entry point; the scaler is called over every
shape 1–5 × 1–5 in both directions and a set of extreme aspects, every
filter. Deterministic, not coverage-guided: the `cargo fuzz` target over
decode + crop + scale + pack above is still to do, and is the place
`pic-scale-safe`'s lack of upstream fuzzing is answered.

**From [ticket 9](restore-state-done.md#review-of-pr-290):**
`query`'s per-output `draw_failed` tells a failed draw from a `clear`
(both show `shows: null`), and is covered end to end only for a restored
image that no longer decodes (`tests/restore.rs`). A failed draw on a
live `set` (a buffer too large for `wl_shm`, out of memory) is covered by
the reply's error and stderr, not yet by a `draw_failed` check; nor does
`query` say *why* a draw failed (stderr does). Worth a test, and a reason
string in `query`, if an agent ever needs to tell the causes apart.

## Resolution

Resolved 2026-09-28. Most of the ticket had landed with the tickets
before it (the audit below says where); this one adds the coverage-guided
fuzz target, the live `set` that cannot be drawn, the one precedence case
left, and records what cannot be tested here. **The suites are not done:
they grow with every later milestone** (transitions, animation, more
formats each bring their own), and [testing.md](../../testing.md) is the
map of where each kind of test lives.

### What landed

- **A `cargo fuzz` target over the whole image path**
  (`crates/scootbg/fuzz/`, entry point `src/image/fuzz.rs`). Arbitrary
  bytes: a 19-byte header picks the mode, filter, fill color, an
  orientation override (or the file's own EXIF) and up to three buffer
  sizes, and the rest is the file. It runs the daemon's own decode
  (`image::decode::decode`) and the daemon's own per-size loop, which
  moved from `daemon::worker::work` into `image::render::render_each` so
  that the fuzzer and the worker share it: every size but the last
  borrows the image (a crop is a copy), the last takes it (cropped in
  place, dropped before its buffer). Sizes are a `u16` a side, 0 to
  65532, with the top three values standing for 23171 (just past
  `wl_shm`'s limit as a square), 2^20 and `u32::MAX`: 1×1, 1×N, primes,
  larger than the image and extreme aspects all come from that. The
  harness asserts what the daemon relies on: a buffer drawn is the size
  asked for, and a size `wl_shm` refuses is never drawn.
  - **The guards are the daemon's.** The pixel budget (`MAX_PIXELS`,
    2^28) at decode and `wl_shm`'s `int32` limit before a buffer exists,
    both unchanged; the harness adds only a throughput cap, skipping sizes
    `wl_shm` would take that are over 2^22 pixels (2^20 in the runs
    below; raised in review), so runs are spent on paths and not on
    writing memory.
  - **Compiled by `#[path]`.** scootbg is a binary, so the target compiles
    `src/color.rs`, `src/image/mod.rs` and `src/image/fuzz.rs` into its
    own crate, unchanged, with the daemon's allocator (`LargeAlloc`, so a
    claimed-but-unwritten buffer costs what it does in the daemon). No
    library target, feature or `cfg` was added to scootbg; the fuzz
    module is `cfg(test)` there.
  - **Out of every build but its own.** The crate has its own
    `[workspace]` and `Cargo.lock`, so `cargo build --workspace`,
    nextest, clippy and the release binary never see it, and `flake.nix`
    leaves `crates/scootbg/fuzz` out of the packages' source (its corpus
    changing must not rebuild them). CI's path classification already
    sends `crates/scootbg/fuzz/*` to the scootbg job, which runs the
    replay test below; the fuzz crate's `README.md` is docs-only.
  - **Replayed on stable in CI.** `image::fuzz::tests` replays the
    committed seed corpus (21 inputs, 6.4 KB) and `regressions/whole/`
    through the same entry point on every `cargo test`; runs every mode,
    filter and orientation over a real JPEG at sizes around it and past
    `wl_shm`'s limit; and checks that every package the fuzz lockfile
    shares with the workspace's is at a version the workspace locks, so a
    dependency bump cannot leave the fuzzer on code the daemon no longer
    runs (checked by hand: `png` set to 0.18.0 in the fuzz lock fails it,
    naming `png`).
- **`query`'s `draw_error`**, additive to protocol 1: the reason while
  `draw_failed` is `true` (the error text stderr gives after "cannot draw
  … on OUTPUT: "), `null` otherwise. `Output::failed` became an
  `Option<String>` (was a `bool`), set by the three places that mark a
  failed draw and cleared where the flag was (a request for the output,
  a `configure`, a new scale); it is allocated only when a draw fails.
  The failed `set`'s reply now points at `draw_error`. It was cheap and
  clean, so it is in rather than deferred: an agent can now tell a
  missing file from an out-of-memory buffer without reading the
  daemon's stderr.
- **A live `set` that decodes but cannot be drawn, end to end**
  (`tests/draw_failed.rs`). A 3840×2160 headless scoot output (the
  harness gained `Session::start_sized`), a color set, then the daemon's
  soft `RLIMIT_AS` lowered (`prlimit`, the hard limit untouched so it can
  be raised back without privilege) to what it maps idle plus 16 MiB, and
  a 1×1 PNG `set` with `--mode center`: the decode fits, the 33 MB
  `wl_shm` buffer does not. The test checks the reply (exit 1, pointing at
  `query`), `draw_failed: true` with `draw_error` naming the buffer's
  `ENOMEM` (`shared memory: Cannot allocate memory (os error 12)`), the
  color still shown, stderr agreeing with `query`, the daemon alive, no
  retry loop, and the next `set` (limit lifted) drawing and clearing
  both. `center` matters: it scales nothing, so the buffer is the draw's
  only large allocation and a fallible one. Repeated 8 times, 8 passes.
  `tests/restore.rs`' restored-but-corrupt case now also checks
  `draw_error` ("… truncated or corrupt …").
- **The precedence case left**
  (`tests/config.rs`,
  `a_set_survives_two_overrides_and_a_new_command_across_a_restart`): a
  section with a table for each output and a `command`, a `set`, a
  restart whose section differs only in `command`: the `set` is restored
  on both outputs, over both tables.
- **Docs:** [docs/scootbg/testing.md](../../testing.md) (how to run the
  suites, what covers what, and the gaps below), the fuzz crate's
  `README.md` (how to run it, what to do with a crash, keeping its
  lockfile in step), `draw_error` in [cli.md](../../cli.md#query), the
  README's protocol notes and Standards, `scootbg query --help`, the
  CHANGELOG.

### The ticket, item by item

- **Unit tests for fit geometry, state round trips, request parsing:**
  already there (`image/fit/tests.rs`, portrait on landscape in
  `fill_crops_the_overflow_centred`, 1×1 and extreme aspects in
  `one_pixel_and_extreme_aspects_stay_inside_and_non_empty`;
  `density/tests.rs` for sizes that round at fractional scales;
  `state/tests.rs`, `state/format`; `protocol/tests.rs`).
- **End to end, colors and images per output at known points:**
  `tests/color.rs` (every pixel, scoot and sway) and `tests/image.rs`
  (`every_fit_mode_on_scoot`: centre, letterbox bars, corners), with
  `scale.rs` and `share.rs`. Done as Rust integration tests rather than a
  shell script: the same pixels, with the harness the rest of the suite
  already has.
- **The precedence table:** every row in `tests/config.rs` since ticket
  10; the two-overrides-and-a-new-`command` case added here.
- **Hotplug on the headless backend:** not possible (below); covered on
  headless sway.
- **Truncated and corrupt files through the decode entry point:** since
  ticket 6 (`image/decode/tests.rs`).
- **The `cargo fuzz` target:** here.
- **`scootbg-mem`'s allocator and the sealed memfd:** since ticket 2
  (`alloc/tests.rs`: every route across the threshold, `alloc_zeroed` on
  both paths, over-aligned blocks to `System`; `tests/global.rs`: threads
  growing and shrinking across the threshold; `shm/tests.rs`:
  `the_memfd_is_sealed_against_resizing`).
- **From ticket 9, `draw_failed` on a live `set`, and a reason:** here.

### The toolchain, and why

**Stable, the project's own (rustc 1.97.1 from the pinned nixpkgs, the
dev shell's), with `cargo fuzz build -s none`.** `-s none` drops the only
flag that needs nightly (`-Zsanitizer`); everything else cargo-fuzz
passes (`-Cpasses=sancov-module`, the `-Cllvm-args` coverage options,
`--cfg fuzzing`) is stable, and the target builds and runs with no
`RUSTC_BOOTSTRAP`. `cargo-fuzz` 0.13.2 comes from the same pinned
nixpkgs for the fuzz command only (`nix shell --inputs-from .
nixpkgs#cargo-fuzz`); nothing was added to the dev shell or CI.

What `-s none` gives up is AddressSanitizer: the fuzzer sees panics,
aborts, timeouts and memory blow-ups, not a silent out-of-bounds read.
That matters only where there is `unsafe`: scootbg, `png`, `image-webp`
and `pic-scale-safe` are `forbid(unsafe_code)`, so for them a memory
error cannot happen and a bounds mistake is a panic the fuzzer does see;
`zune-jpeg`'s `unsafe` (its SIMD kernels) is fuzzed upstream, under ASan.
A nightly or `RUSTC_BOOTSTRAP` build for ASan was not needed for this
ticket and was not used.

### The fuzz runs

On this container (4 vCPUs, Intel Xeon @ 2.80GHz, 15 GB, Linux
6.18.44), the target built by `cargo fuzz build -s none` with rustc
1.97.1, from this branch's working tree before its commit. The `-a`
run's sources (`color.rs`, `image/*.rs`, `scootbg-mem`, the target and
its lockfile, by `sha256sum`) are the committed ones exactly; the release
run's differ only in comments and `rustfmt::skip` attributes in
`image/fuzz.rs` and `fuzz_targets/whole.rs`, and one seed was added
after it. Both runs used `-fork=3
-max_len=8192 -timeout=30 -rss_limit_mb=4096`, a scratch corpus first
and the seed corpus second, and wrote artifacts outside the tree.

| Run | Wall | CPU (3 workers) | Executions | Edges (`cov`) | Features (`ft`) | Corpus | Crashes | OOMs | Timeouts | Slow units |
|---|---|---|---|---|---|---|---|---|---|---|
| release semantics | 2747 s | ≈ 2.3 h | 7,245,250 | 4454 | 15163 | 3535 | 0 | 0 | 1 | 10 |
| `-a` (debug assertions: overflow checks in every crate) | 1200 s | ≈ 1.0 h | 4,495,577 | 4567 | 15868 | 2954 | 0 | 0 | 0 | 1 |

(`cov` counts instrumented edges, so the two builds' numbers are not
comparable: `-a` adds the overflow checks' own.) **No crash, no panic, no
out-of-memory, in either run.** Line coverage of scootbg's image path
over the release run's corpus, measured apart with an
`-Cinstrument-coverage` build of the same target and `llvm-cov` 21.1.8:
83.9% of the lines in `color.rs` and `image/` (926 lines, 149 missed),
from 81.3% for the seed corpus alone. What stays missed is code the
entry point never calls (the error types' `Display`, `decode_file`'s
open, `color.rs`'s parsing) and defensive returns the checks before
them make unreachable (`scale::scale`'s size refusals, which `render`
pre-empts; `pack`'s range errors; the PNG and WebP layout mismatches).
One real gap showed: no input carried a little-endian EXIF block, so
`exif.rs`'s `II` reads never ran; a seed with one was added and reaches
them.

**The one timeout and every slow unit are the same input class, and not
a defect:** an extended (VP8X) WebP whose canvas is at the pixel budget
(the timeout: 32×8,257,568, 264 megapixels, in a 382-byte file) with a
small first frame. `image-webp` builds the whole canvas for an animated
file's first frame, and scootbg then decodes and scales an image of the
budget's size, which is what the budget allows. Run once through an
uninstrumented release build of the same target
(`-runs=1 -print_final_stats=1`): the timeout input takes 4.5 s with a
Lanczos `fit` (1.1 s with `center`, which scales nothing) and peaks at
1068 MB RSS; the ten slow units take 0.4–3.0 s and peak at 310–820 MB.
The sanitizer-coverage instrumentation is what makes the first one pass
30 s under the fuzzer. That is the cost of any in-budget image the
daemon accepts (the budget's own figure is 805 MB of RGB), paid on the
worker thread, not the loop. The canvas `Vec` is an infallible
allocation inside `image-webp`, the class
[ticket 6](images-decode-and-fit-done.md) already records for the
decoders' working memory. None was committed as a regression file: none
fails, and each would take minutes in a debug-build test.

**Checked while reading, and not reachable:** `image-webp` sizes an
animation canvas as `width * height * 4` in `u32` (`decoder.rs`,
`read_frame`), which would wrap past 2^30 pixels; the pixel budget (2^28)
is checked on the canvas size before `read_image`, so the product stays
under 2^30. The `-a` run, where that multiplication panics on overflow,
found nothing there either.

### Found along the way

- **`#[path]` modules and `rustfmt`.** A module loaded by `#[path]` is a
  "mod-rs" file, so its children resolve beside it: the fuzz crate's
  `cargo fmt` looked for `src/tests.rs` for `color.rs`'s `cfg(test)`
  child and failed. The three `#[path]` modules carry
  `#[rustfmt::skip]` (they are formatted with scootbg), and nothing is
  lost in the build, where `cfg(test)` children are not compiled.
- **An empty `regressions/whole/`** would not survive a checkout (git
  keeps no empty directories), and the replay test reads it: it holds a
  `.gitkeep`, and the test skips dotfiles.
- **The out-of-memory draw needs a mode that scales nothing.** Under the
  same lowered address-space limit, a `fill` of the same 1×1 PNG aborts
  the daemon (checked by hand: `memory allocation of 24883200 bytes
  failed`, exit 134): the 3840×2160 RGB output `pic-scale-safe` allocates
  inside `resize_rgb8` fails before `wl_shm`'s fallible `mmap` is
  reached. The scaler has no API that writes into a caller's buffer, so
  scootbg cannot make that allocation fallible from its own code. That is
  the documented class (the scaler's and decoders' working memory is
  infallible; [ticket 6](images-decode-and-fit-done.md)), not new, but
  it is why the test uses `center`, and why the out-of-memory case the
  daemon survives is the buffer and not the scaler. Now its own backlog
  item, waiting on a decision: [scaler-oom-abort.md](../scaler-oom-abort.md).

### For the next tickets

- **A scheduled fuzz job in CI, proposed and not added.** It needs no new
  toolchain: the stable rustc CI already has, `cargo-fuzz` from the
  pinned nixpkgs (`nix shell --inputs-from . nixpkgs#cargo-fuzz`), and a
  C++ compiler for `libfuzzer-sys` (the runner's). A nightly or weekly
  `schedule:` job running `cargo fuzz run -s none whole` for 20–30
  minutes from the seed corpus, failing on a crash and uploading the
  artifact, would keep the target honest as the image path grows
  (animation, more formats). Whether that CI time is wanted is the
  user's call; the replay test already runs the corpus on every PR.
- **AddressSanitizer** would need nightly (or `RUSTC_BOOTSTRAP`) and
  only adds coverage for `zune-jpeg`'s SIMD, which is fuzzed upstream
  under ASan; not worth a toolchain exception now. Revisit if a
  dependency with `unsafe` and no upstream fuzzing joins the path.
- **Milestone 2** (transitions, animated images) and **more formats**
  each extend the fuzz target: an animated image's later frames, and
  each new decoder, go through the same entry point, with seeds for
  them.
- **`draw_error`'s wording** is text for people and agents, not an
  enum. If an agent ever needs to branch on the cause, add a stable
  `draw_error_kind` (`decode`, `memory`, `too-large`, …) beside it,
  additively.
- **Hotplug on `--tty`**: scootbg has not been run through a real
  monitor unplug on the dev VM; the sway coverage and scoot's own
  `--tty` record stand in until someone with the VM does.

### Benchmarks

The daemon changed in the draw path (the per-size loop moved into
`render_each`, and a failed draw keeps its reason), so the idle and
`set` rows were run for the base (`origin/main` at `6343ac3`, built in a
separate worktree and target directory) and this branch, on this
container, same scoot, `scripts/scootbg-bench/bench.py run --only
set,idle --daemons scootbg --rounds 3 --idle-secs 30 --window-secs 30`,
then the `set` row alone, 5 rounds, interleaved base, head, base, head.
`compare` finds **no regression beyond the margin** in any of the three
pairs:

| Row | base | head |
|---|---|---|
| Idle RSS, 1× 1080p, image (MiB) | 12.2 | 12.2 |
| Idle PSS above the floor, 2× 4K, image (MiB) | 3.2 | 3.2 |
| Idle RSS, 1× 1080p, color (MiB) | 3.8 | 3.9 |
| Idle wakeups / CPU in 60 s, every case | 0 / 0.0 ms | 0 / 0.0 ms |
| Peak PSS, live change to the JPEG, 1× 4K (MiB) | 100.7 | 101.3 |
| Set: latency to the JPEG (ms), run 1 / 2 / 3 | 648 / 692 / 698 | 718 / 710 / 688 |
| Set: latency to a color (ms), run 1 / 2 / 3 | 17.1 / 19.8 / 23.4 | 18.1 / 24.0 / 26.4 |
| Release binary, stripped (bytes) | 1,684,328 | 1,684,328 |

Medians, three rounds for the first run and five for the others; the
JPEG's range overlaps in every pair (base 591–806, head 603–869 ms).
Against the published [2026-09-28 idle run](../../bench/2026-09-28-idle-scoot/table.md),
base and head alike show the same 0.8 MiB more PSS on the same ten rows:
that run had every daemon up at once, sharing library pages, and these
ran scootbg alone, so it is the method, not the code. The CPU here is
reported as 2.80 GHz against that run's 2.10 GHz, which is why this
compares base with head on this machine rather than head with the
published numbers.

### Verified where

On the container above, for the tree committed with this record.

- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
  SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
  nextest run -p scootbg -p scootbg-mem`: 434 passed, 2 skipped (the
  `#[ignore]`d benchmarks).
- `cargo test -p scootbg -p scootbg-mem` (same variables): every suite
  passed; unit tests 332 (2 ignored), `config.rs` 24, `restore.rs` 10,
  `draw_failed.rs` 1, `scootbg-mem` 17 + 4.
- `soft-egl cargo nextest run --workspace`: 2649 passed, 28 skipped.
- `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D
  warnings`, `cargo fmt --check -p scootbg -p scootbg-mem`, the same for
  `-p scoot`, `RUSTFLAGS="-D warnings" cargo build --release -p scootbg`,
  `cargo build --workspace`, and CI's check for a `libc` crate in
  scootbg's tree: clean. In the fuzz crate, `cargo clippy --all-targets
  -- -D warnings` and `cargo fmt --check`: clean.
- `nix flake check --no-build`: all checks evaluate; the scootbg
  package's source (`nix eval .#packages.x86_64-linux.scootbg.src`)
  holds no `crates/scootbg/fuzz`.
- The README's fuzz command, as written, builds and runs.
- `tests/draw_failed.rs` 8 times in a row: 8 passes.

### Not verified, and why

- **`nix build .#scootbg` in the sandbox:** refused here (a
  `/homeless-shelter` exists, which this environment keeps); CI's
  `nix flake check` builds it.
- **`--tty`, the dev VM, a GPU:** not reachable from this container.
- **CI itself**, including that the scootbg job's replay test runs from
  a fresh checkout (it needs `regressions/whole/.gitkeep`, which is
  committed): first exercised by this branch's CI run.

## Review of PR #317

No blocking findings; the low ones, fixed in commits after `53669ac`:

1. **The fuzzer ran inputs on a bigger stack than the daemon.** libFuzzer
   calls the target on its 8 MiB main thread; the daemon decodes on a
   `std::thread` with std's 2 MiB default. An input needing 2–8 MiB of
   stack would kill the daemon and pass the fuzzer. Now one constant,
   `image::DECODE_STACK` (2 MiB), sets the worker's stack explicitly
   (`Builder::stack_size`, so `RUST_MIN_STACK` no longer changes it) and
   `image::fuzz::whole_path` runs each input on a scoped thread of that
   size, re-raising its panic. The whole corpus of both runs above (4068
   inputs) replayed through the new harness (`-runs=0`, the 2^22 cap):
   no crash, no overflow.
2. **The lockfile check compared against the whole workspace lock**, not
   scootbg's graph: the workspace locks two `miniz_oxide` (0.8.9 for
   `png`, 0.9.1 for `flate2`, and scootbg reaches both) and two `rustix`,
   so a fuzz lock whose `png` took the other `miniz_oxide` would have
   passed. It now follows the dependency edges of both locks, from
   `scootbg` and from `scootbg-fuzz`: every package both reach must be at
   a version scootbg reaches, and a dependency both copies have must
   resolve to the same version (features may add or drop one, which is
   why `cc`'s `jobserver` and `serde`'s `serde_derive` do not count).
   `a_dependency_scootbg_does_not_resolve_to_is_caught` plants both:
   `png` → `miniz_oxide 0.9.1` gives `png 0.18.1: fuzz takes miniz_oxide
   0.9.1, scootbg 0.8.9`; `rustix` at 0.38.44 gives `rustix: fuzz
   0.38.44, scootbg ["1.1.4"]` and `scootbg-mem 0.1.0: fuzz takes rustix
   0.38.44, scootbg 1.1.4`.
3. **`tests/draw_failed.rs`:**
   - the lowered limit is a `Drop` guard, so a panic still lifts it;
   - a premise probe: a finite hard `RLIMIT_AS` below what the daemon
     maps, the 33 MB buffer and twice the margin (so both the failure
     and the retry after it fit) skips with a message. Checked by
     running the test binary under `prlimit --as=H:H`: at 75,000,000 it
     prints `skipped -- …` and passes; unlimited and 400,000,000 run and
     pass. Below about 70 MB the harness itself cannot start scoot and
     fails there, before the test's premise; at 90 MB the retry's
     `set` loses its connection to the daemon. The compositor inherits
     the same limit from the test run, and most likely cannot map the
     33 MB buffer (not traced): run by hand with only the daemon under a
     90 MB hard limit, the same steps pass. A whole-run `ulimit -v` that
     small is outside what this test can speak for;
   - the glibc arena premise, in the module docs, checked once by
     `strace -f -e trace=mmap,munmap,clone3` on the test's steps (a
     3840×2160 headless scoot, the debug daemon, VmSize 76,236 kB): the
     state saver thread of the color `set` reserves its arena before the
     limit (`mmap(NULL, 134217728, PROT_NONE, …)`, trimmed to 64 MiB);
     under the limit, the decoding thread reuses that free arena and a
     cached stack, and its one large call is the buffer's `mmap(NULL,
     33177600, …, MAP_SHARED, 9, 0) = -1 ENOMEM`. Under a finite hard
     limit, where no arena could ever be reserved, the trace shows the
     reservation failing again and again and glibc sharing the main
     arena instead: the `set` after that succeeded.
4. **Nits:** the fuzz crate's `[profile.release]` comment now says it
   sets only `debug = 1` and that nothing it leaves out changes
   behaviour; `cli.md`'s query paragraph lost the "stderr says why"
   parenthetical that `draw_error` makes redundant, rewrapped.
5. **`MAX_FUZZ_PIXELS` raised to 2^22**, so 1080p and 1440p outputs are
   drawn. Two-minute check, `-fork=3` from the seed corpus alone, fresh
   scratch corpus each: at 2^20, 981,268 executions, `cov` 3577, `ft`
   9185; at 2^22, 610,189 executions, `cov` 3690, `ft` 9987. A third
   fewer runs for more coverage in the same time: kept.
6. **The scaler's abort** is its own backlog item,
   [scaler-oom-abort.md](../scaler-oom-abort.md), open and waiting on a
   decision between accepting it, a probe, a scoot-sh fork of
   `pic-scale-safe` with a destination-slice entry point, and another
   scaler.

Verified at `8d84be8` on the same container: `nextest` for scootbg and
scootbg-mem (with scoot and sway required) 435 passed, 2 skipped;
`cargo test` every suite ok (unit tests 333); clippy `-D warnings`, fmt,
the release build (`-D warnings`, 1,684,328 bytes, unchanged), no `libc`
crate, `cargo build --workspace`; the fuzz crate's clippy and fmt. The
worker's stack is now set rather than defaulted, to the same 2 MiB, so
the `set` row was run again (5 rounds) against the base run above:
JPEG 633 ms against 698, color 17.1 against 23.4, peak PSS 101.6 against
100.9 MiB, `compare` 0 regressions.
