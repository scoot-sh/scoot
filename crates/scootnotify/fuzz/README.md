# scootnotify's fuzz targets

Three `cargo fuzz` targets over the parsers of outside input: the
`Notify` body (`notify`), the markup parser (`markup`), and the
`image-data` validator (`image-data`). Test tooling only: never part
of the shipped binary, and never a member of the repository's workspace
(the `[workspace]` table in `Cargo.toml` makes this its own), so
`cargo build --workspace`, nextest, clippy and the flake's builds never
compile it. How to run it: the CI job runs all three for a fixed budget
from a fixed seed; by hand:

```sh
nix shell --inputs-from . nixpkgs#cargo-fuzz --command \
  cargo fuzz run -s none notify fuzz/corpus/notify fuzz/regressions/notify
```

The targets call the crate's own `fuzz_check` functions (over the
library half of the crate, by path), so a refactor that moves a parser
breaks here first. The seeds in `corpus/` are written by
`cargo run -p scootnotify --example gen_corpus` — real marshalled
`Notify` bodies, markup texts and `image-data` structs, committed so a
run is the same run every time. Past findings live in `regressions/`.

## What each target checks

- **`notify`**: arbitrary bytes as a `Notify` body. Parsing refuses
  (a short read, a misshapen body, trailing bytes) or accepts within
  every documented cap (`app_name` 128 B, `summary` 1 KiB, `body`
  8 KiB, at most 16 action pairs, short hint strings cut), never a
  panic.
- **`markup`**: arbitrary bytes as a body (skipped unless UTF-8).
  Parsing yields at most 64 spans whose text the input carried
  (entities only shrink), never a panic.
- **`image-data`**: arbitrary bytes as an `(iiibiiay)` struct value.
  Validation refuses (odd sides, wrong bits, disagreeing channels, a
  short rowstride, short pixels) or downscales within the 128 px icon
  bound with exactly its pixels, never a panic.
