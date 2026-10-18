# scootlaunch's fuzz targets

Two `cargo fuzz` targets over the parsers of outside input: the
desktop entry parser (`parse`) and the Exec expander (`exec`). Test
tooling only: never part of the shipped binary, and never a member of
the repository's workspace (the `[workspace]` table below makes this
its own), so `cargo build --workspace`, nextest, clippy and the
flake's builds never compile it. How to run it: the CI job runs both
for a fixed budget from a fixed seed; by hand:

```sh
nix shell --inputs-from . nixpkgs#cargo-fuzz --command \
  cargo fuzz run -s none parse fuzz/corpus/parse fuzz/regressions/parse
```

The seeds in `corpus/` are verbatim `.desktop` files from NixOS, Fedora
and Debian packages. They stay under their packages' licenses, ship as
test inputs only, and are never built into the binary.

## What each target checks

- **`parse`**: arbitrary bytes as a desktop file. Parsing refuses
  (malformed UTF-8, an overlong line, too many groups, no
  `[Desktop Entry]`) or accepts without a panic; whatever it accepts
  re-reads within bounds (at most 32 groups, every string within
  1 KiB, lists split on `;`). Compiles `src/index/parse.rs` by
  `#[path]`, and uses nothing but `std`.
- **`exec`**: arbitrary bytes as an `Exec` value (skipped unless
  UTF-8). Expansion refuses or accepts without a panic, in both the
  strict and the lenient mode; an accepted argv is non-empty, its
  executable holds no `=`, and an expansion never re-splits (one
  argument per input argument, plus `--icon` and the terminal
  prefix). Compiles `src/index/exec.rs` by `#[path]`, and uses
  nothing but `std`.

The seed corpora are real desktop files: a NixOS desktop's
`/run/current-system/sw/share/applications` (the dev VM), a Fedora
and a Debian `/usr/share/applications` sample (fetched for the
ticket; see the report), plus every quoting example in §7 of the
Desktop Entry Specification. Past findings live in `regressions/`.
