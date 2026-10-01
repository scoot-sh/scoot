# scootbar's fuzz targets

Three `cargo fuzz` targets over the parsers of outside input: two the
clock's, one the update payload of the `exec` and `push` modules. Any
panic is a finding: the bar's release profile is `panic = "abort"`, so a
panic there kills the bar.

- **`format`**: arbitrary bytes as a `--clock-format` (the first byte picks
  a zone offset and instants). Parsing must accept or refuse, never panic;
  an accepted format must render any instant, `i64::MIN` to `i64::MAX`, in
  any offset, with no control character in the output and within a length
  bound.
- **`tzif`**: arbitrary bytes as a zone file (`/etc/localtime`, or what
  `TZ` names) and as a POSIX TZ string. A zone read must answer any instant
  with an offset within a day and two hours of UTC and a printable
  abbreviation of at most 8 bytes.
- **`payload`**: arbitrary bytes as an `exec` module's output line, in both
  formats, and, where they are JSON, as a `msg set` value
  (`src/modules/payload.rs`). Refused or accepted without a panic; an
  accepted update holds text and tooltip within 256 bytes, with no control
  character and no space at either end; a refused one changes nothing. It
  compiles `class.rs` and `payload.rs` by `#[path]`, and uses `serde_json`
  (the only dependency of this crate besides `libfuzzer-sys`).

The first two compile scootbar's own `src/modules/clock/tzif.rs` and `format.rs` by
`#[path]`, unchanged (`fuzz_targets/common.rs`); they use nothing but
`std`. What each target checks is written once, in scootbar's
`src/modules/clock/fuzz.rs`, compiled here the same way, so each target is
one line. This crate is its own workspace, with its own `Cargo.lock`: it is
never built by `cargo build --workspace`, nextest, clippy or the flake, and
nothing here reaches the shipped binary.

**In CI**, on every scootbar change, the `scootbar` job builds the targets
and runs them for a fixed budget: 1,000,000 runs of `format` and
5,000,000 of `tzif` and 2,000,000 of `payload`, from `-seed=1`, over the seed corpus and
`regressions/` (about 35 s). Building them is what keeps the `#[path]`
includes from rotting: a change that compiles in scootbar but not here
fails there. A finding's input is printed in base64 in the job's log.
Before building, the step runs `cargo fetch --locked` on this workspace
(cargo-fuzz has no `--locked` to pass on), so a `Cargo.lock` that no
longer matches `Cargo.toml` fails the job instead of being rewritten.
**On every `cargo test`**, scootbar's
`modules::clock::fuzz::tests` replay the seed corpus and every file in
`regressions/` through the same checks on the stable toolchain, and the
property tests (`modules/clock/format/tests.rs`,
`modules/clock/tzif/tests.rs`) run a bounded version of them.

## Running it

The pinned stable toolchain with no sanitizer (`-s none`, as scootbg's);
`cargo-fuzz` comes from the pinned nixpkgs for this command only. From the
repository root:

```sh
devenv shell -- nix shell --inputs-from . nixpkgs#cargo-fuzz --command bash -c '
  cd crates/scootbar &&
  mkdir -p /tmp/scootbar-corpus-format /tmp/scootbar-corpus-tzif &&
  cargo fuzz run -s none format /tmp/scootbar-corpus-format fuzz/corpus/format \
    fuzz/regressions/format -- -max_len=512 -timeout=10 -max_total_time=300 &&
  cargo fuzz run -s none tzif /tmp/scootbar-corpus-tzif fuzz/corpus/tzif \
    fuzz/regressions/tzif -- -max_len=70000 -timeout=10 -max_total_time=300'
```

- **`-max_len`**: 512 bytes covers any format (they are capped at 256);
  70,000 covers the TZif reader's 64 KiB cap and a little past it, so the
  refusal of an oversized file is reached too.
- **Give it a scratch corpus first**, as for scootbg: libFuzzer writes what
  it finds into the first directory, and `fuzz/corpus/` is the committed
  seed (real zone files, fat and slim, and a few formats and POSIX rules).
- A crash goes to `fuzz/regressions/<target>/` once fixed, named for what
  it was, so every later run replays it, fuzzing or not.

## Runs

| When | Commit | Target | Runs | Time | Findings |
| --- | --- | --- | --- | --- | --- |
| 2026-09-29 | see the PR | `format` | 11,738,471 | 301 s | none |
| 2026-09-29 | see the PR | `tzif` | 113,636,649 | 301 s | none |
