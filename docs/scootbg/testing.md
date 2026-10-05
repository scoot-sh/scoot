# Testing scootbg

How scootbg is tested, how to run it, and what is not covered and why.
The suites grow with every milestone; this page says where each kind of
test lives, not every test.

## Running it

From the repository root, in the dev shell, with `scoot` built beside
scootbg (`cargo build -p scoot`: the end-to-end tests start it):

```sh
SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1 \
  devenv shell -- soft-egl cargo nextest run -p scootbg -p scootbg-mem
devenv shell -- cargo test -p scootbg -p scootbg-mem
devenv shell -- cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings
devenv shell -- cargo fmt --check -p scootbg -p scootbg-mem
```

| Variable | Effect |
|---|---|
| `SCOOTBG_TEST_SCOOT` | the `scoot` binary to run; default, the one beside the test's `scootbg` |
| `SCOOTBG_REQUIRE_SCOOT` | no `scoot` is a failure, not a skip (CI sets it) |
| `SCOOTBG_TEST_SWAY` | the `sway` binary to run; default, `sway` on `PATH` |
| `SCOOTBG_REQUIRE_SWAY` | no `sway` is a failure, not a skip (CI sets it) |

CI gets sway from `nix build --inputs-from . nixpkgs#sway`; the same
command gives it locally.

## What covers what

- **Unit tests**, in a `tests.rs` beside each module: fit geometry for
  every mode against odd sizes, portrait on landscape, 1×1 and extreme
  aspects (`image/fit`); scales that round at fractional values
  (`density`); state-file round trips and hostile state files (`state`);
  request parsing and replies (`protocol`); every decoder cut at every
  length and corrupted at random, and the scaler over every small shape
  (`image/decode`, `image/scale`); images with a side too long to scale
  (`image/long_axis_tests.rs`); the daemon's reply logic over a fake
  compositor (`daemon`).
- **The allocator and the `wl_shm` buffer** (`crates/scootbg-mem`):
  allocations across the 128 KiB threshold both ways, `alloc_zeroed` on
  both paths, alignments above a page, several threads at once, and a
  sealed memfd that refuses to be truncated.
- **End to end** (`crates/scootbg/tests/`), each against a real
  `scoot --headless` and, where scoot cannot do it, a headless sway:
  colors and images checked by screenshot pixels (`color.rs`,
  `image.rs`, `scale.rs`, `share.rs`), outputs and their surfaces
  (`outputs.rs`, `hotplug.rs`), the daemon's lifecycle and hostile
  clients (`daemon.rs`), restore (`restore.rs`), `apply-config` and the
  precedence of a `set` over scoot's `[wallpaper]` section (`config.rs`,
  `scoot_config.rs`), and a live `set` that decodes but cannot be drawn
  (`draw_failed.rs`: out of memory for the buffer, and a side too long to
  scale; `query`'s `draw_failed` and `draw_error`).
- **Fuzzing**: a `cargo fuzz` target over the whole image path, decode,
  crop, scale and pack, on the stable toolchain. How to run it, and what
  to do with a crash: [`crates/scootbg/fuzz/README.md`](../../crates/scootbg/fuzz/README.md).
  Its seed corpus and every past crash are replayed by an ordinary test
  (`image::fuzz::tests`), so CI covers them without cargo-fuzz.

## Not covered, and why

- **Hotplug on headless scoot.** scoot's headless backend makes its
  outputs at start-up (`--outputs N`) and cannot add or remove one at
  runtime, over IPC or otherwise. Outputs coming and going are covered on
  headless sway instead (`hotplug.rs`, and the hotplug halves of
  `color.rs`, `image.rs` and `share.rs`). On `--tty`, scoot's own
  handling of a hotplug (layer-shell surfaces re-anchored, `wl_output`
  withdrawn, see [backends](https://scoot-sh.github.io/scoot/scoot/backends.md)) is the
  compositor's record; scootbg itself has not been run through a real
  hotplug on `--tty` yet.
- **A buffer too large for `wl_shm`, end to end.** That needs an output
  of over 536 million pixels, which no compositor here can be asked for
  without allocating it too. The refusal is unit-tested
  (`scootbg-mem`'s `sizes_past_int32_are_refused`) and fuzzed (sizes
  past the limit are in the fuzz input), and the end-to-end draw
  failure is the other one the daemon can meet, out of memory for the
  buffer (`draw_failed.rs`).
- **Memory errors inside `zune-jpeg`'s SIMD code.** The fuzzer runs
  without a sanitizer (`-Zsanitizer` is nightly-only), so it finds panics
  and crashes, not a silent out-of-bounds read. scootbg, `png`,
  `image-webp` and `pic-scale-safe` are `forbid(unsafe_code)`;
  `zune-jpeg` is fuzzed upstream.
