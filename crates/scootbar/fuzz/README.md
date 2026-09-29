# scootbar's fuzz targets

Two `cargo fuzz` targets over the clock's parsers of outside input. Any
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

Both compile scootbar's own `src/modules/clock/tzif.rs` and `format.rs` by
`#[path]`, unchanged (`fuzz_targets/common.rs`); they use nothing but
`std`. This crate is its own workspace, with its own `Cargo.lock`: it is
never built by `cargo build --workspace`, nextest, clippy or the flake, and
nothing here reaches the shipped binary. The stable property tests in the
crate (`modules/clock/format/tests.rs`,
`modules/clock/tzif/tests.rs`) run a bounded version of the same checks in
every CI run.

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
- A crash goes to `fuzz/regressions/<target>/` once fixed, so every later
  run replays it.

## Runs

| When | Commit | Target | Runs | Time | Findings |
| --- | --- | --- | --- | --- | --- |
| 2026-09-29 | see the PR | `format` | 11,738,471 | 301 s | none |
| 2026-09-29 | see the PR | `tzif` | 113,636,649 | 301 s | none |
