# Config parser spike: `toml` vs `basic-toml` vs hand-rolled

Measured 2026-09-29 for
[config-cli-and-reload](../backlog/config-cli-and-reload.md). The schema
under test is the real one (nested `[bar]`/`[colors]`/`[clock]` tables,
`left`/`center`/`right` arrays, every table `deny_unknown_fields`):
`GOOD` in the spike crate below parses the full planned `bar.toml`.

**Outcome: the workspace's existing `toml` (v1, as the `scoot` crate
uses).** It adds no new package to the tree, parses the schema in ~7 µs,
and refuses malformed input with an error naming the key, never a panic.
The evidence, not the assertion, follows.

## What was run

A throwaway crate (not a workspace member, not committed):
`/tmp/opencode/parser-spike` (removed after; this file is the record).
Two binaries sharing one schema module, one parsing with `toml 1.1.6`,
the other with `basic-toml 0.1.10`, each over the same inputs:

- `GOOD`: the full planned `bar.toml` (~600 bytes).
- unknown key (`hieght` in `[bar]`), bad value (`height = "tall"`),
  malformed (`[bar` unclosed).
- bench: 2000 parses of `GOOD`, timed as a batch, release build,
  3 repetitions.

```
$ CARGO_TARGET_DIR=/tmp/opencode/parser-spike-target cargo build --release
$ ./target/release/spike-toml        # errors
$ ./target/release/spike-basic       # errors
$ ./target/release/spike-toml bench  # 3x, same for spike-basic
```

## Results

| | `toml` 1.1.6 | `basic-toml` 0.1.10 |
|---|---|---|
| Parse `GOOD` (release, mean of 3 runs of 2000) | 6.6–6.8 µs/parse | 11.6–12.6 µs/parse |
| Unknown key | names it: ``unknown field `hieght`, expected one of `edge`, …`` with the line and column | names it: ``unknown field `hieght`, … for key `bar` …`` |
| Bad value | ``invalid type: string "tall", expected u32`` with the line | ``invalid type: string "tall", expected u32 for key `bar.height` …`` |
| Malformed | ``unclosed table, expected `]` `` with the line | ``expected a right bracket, found a newline …`` |
| Deps beyond `serde` | `toml_datetime`, `toml_parser`, `toml_writer`, `winnow` (+ `serde_spanned`) | none (`serde` only) |
| New packages for this workspace | **0** (`scoot` already pins exactly this `toml`; it is in `Cargo.lock`) | 1 (`basic-toml` itself) |
| Toy release binary (schema + `serde` + the parser) | 878,288 bytes | 751,808 bytes |

Raw error lines (exact output, `spike-toml`):

```
toml unknown-key: Err: TOML parse error at line 3, column 1
  |
3 | hieght = 28
  | ^^^^^^
unknown field `hieght`, expected one of `edge`, `height`, `margin`, `font`, `font-size`, `padding`, `spacing`
toml bad-value: Err: TOML parse error at line 3, column 10
  |
3 | height = "tall"
  |          ^^^^^^
invalid type: string "tall", expected u32
toml malformed: Err: TOML parse error at line 1, column 5
  |
1 | [bar
  |     ^
unclosed table, expected `]`
```

(`spike-basic`'s equivalents name the same keys on one line each; see the
table.)

## Reading

- **Time is not a decider**: the file parses once per start and once per
  reload; 7 vs 12 µs is nothing against a 16 ms frame.
- **Failure behavior ties**: both name the offending key and never panic;
  `toml`'s multi-line form with the source line reads better on stderr.
- **Size decides, weakly**: `basic-toml` is ~126 KB lighter in the toy,
  but it would put a *second* TOML parser in the workspace next to the
  `toml` that `scoot` already depends on, while `toml` adds no new
  version to the tree at all. One parser per workspace beats a smaller
  second one.
- **Hand-rolled is rejected**: it would save on the order of 100 KB but
  reimplements `deny_unknown_fields`, dotted keys, arrays and error paths
  by hand — spec drift and a bespoke failure surface for the bar's most
  user-facing input. Not worth it for a file parsed twice a session.

`scootbar` therefore depends on `toml` (workspace version) plus the
`serde`/`serde_json` the control socket needs anyway. The real binary
delta is measured in the implementing PR against the 896,600-byte
release baseline taken on this branch before the change.
