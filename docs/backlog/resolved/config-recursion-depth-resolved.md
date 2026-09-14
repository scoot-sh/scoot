---
title: "Config parsing has no recursion-depth guard (LOW) \u2014 RESOLVED 2026-09-13 (investigation + regression tests, PR #21): at `toml` 1.1.6 the premise doesn't hold at the default stack rlimit (8 MiB, unmodified `ulimit -s`) \u2014 the crate guards nesting itself, at 80 levels, and anything deeper lands in exactly the \"log and fall back to defaults\" path the entry worried it would bypass. No production code change; a guard of flexwm's own would be a second, worse bound on top of a precise one. The guard bounds the depth, though; it does not make the parse free, so the conclusion is scoped to that 8 MiB budget \u2014 a *debug* build launched under `ulimit -s 2048` does still abort on the worst case (measured, below)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Config parsing has no recursion-depth guard (LOW) — RESOLVED 2026-09-13 (investigation + regression tests, PR #21): at `toml` 1.1.6 the premise doesn't hold at the default stack rlimit (8 MiB, unmodified `ulimit -s`) — the crate guards nesting itself, at 80 levels, and anything deeper lands in exactly the "log and fall back to defaults" path the entry worried it would bypass. No production code change; a guard of flexwm's own would be a second, worse bound on top of a precise one. The guard bounds the depth, though; it does not make the parse free, so the conclusion is scoped to that 8 MiB budget — a *debug* build launched under `ulimit -s 2048` does still abort on the worst case (measured, below).

~~Config parsing has no recursion-depth guard (LOW)~~ — RESOLVED
2026-09-13 (investigation + regression tests, PR #21): at `toml` 1.1.6 the
premise doesn't hold at the default stack rlimit (8 MiB, unmodified
`ulimit -s`) — the crate guards nesting itself, at 80 levels, and anything
deeper lands in exactly the "log and fall back to defaults" path the entry
worried it would bypass. No production code change; a guard of flexwm's own
would be a second, worse bound on top of a precise one. The guard bounds the
depth, though; it does not make the parse free, so the conclusion is scoped
to that 8 MiB budget — a *debug* build launched under `ulimit -s 2048` does
still abort on the worst case (measured, below).
Original diagnosis, left as written: the `toml` stack has no explicit guard
against deeply nested input; a maliciously deep config could
stack-overflow-abort the process rather than hit the module's normal "log
and fall back to defaults" path. Requires the user's own config file, so low
priority.

**What the crate actually does.** `toml` 1.1.6 (over `toml_parser` 1.1.3 and
`winnow` 1.0.4) bounds depth in two independent places, both hard-coded at
80: a `RecursionGuard` around combined inline-table/array nesting
(`toml/src/de/parser/mod.rs:37,58,71`), which the parser consults — it
returns `false` past the limit and `on_array_open`/`on_inline_table_open`
switch to the iterative `ignore_to_value_close` skip instead of recursing
(`toml_parser/src/parser/document.rs:796,962,1480`) — and a separate cap on
dotted-key/table-header path segments (`toml/src/de/parser/key.rs:66`). Both
are active unless the crate's `unbounded` feature is on; it is not, but the
command matters: `unbounded = []` enables nothing else, so `cargo tree
-e features` prints the identical `default, display, parse, serde, std`
whether it's on or off and can't be used to check this. `cargo tree -p
flexwm --target all -f "{p} | {f}"` is the one that actually shows it —
`toml v1.1.6+spec-1.1.0 | default,display,parse,serde,std`, no `unbounded`
suffix — confirmed by building a probe with the feature on and watching the
suffix appear.

**Evidence.** Scratch harness (kept out of the tree; the cases that matter
are now tests in `crates/flexwm/src/compositor/config.rs`), aarch64 on both
macOS 26 and the Linux dev VM, `ulimit -s` 8192 KiB on the Linux dev VM and
8176 KiB on macOS (the actual default there, not 8192 -- doesn't move any
number below, since every macOS figure was measured on an explicitly-sized
spawned thread rather than against the main-thread rlimit):
- *Boundary*, through flexwm's real `toml::from_str::<FileConfig>` path: 80
  levels parse and are then rejected by `deny_unknown_fields` ("unknown
  field a"); 81 are refused by the crate (`cannot recurse further; max
  recursion depth met` for inline tables and arrays, `recursion limit` for
  dotted keys, `[a.a…]` headers and `[[a.a…]]` headers). Identical on both
  platforms. A 1,000,000-level file (4 MB) returns the same clean error in
  milliseconds, no crash.
- *Counterfactual*, the same harness built with `toml/unbounded`, Linux, main
  thread, `ulimit -s 8192`, one process per cell over the depth ladder
  {1k, 2k, 5k, 10k, 20k, 50k, 100k, 200k}. Every nesting form does eventually
  abort with `fatal runtime error: stack overflow` (SIGABRT, exit 134), which
  is the point — the crate's guard is load-bearing, not incidental — but
  *where* it aborts varies by an order of magnitude between forms and by
  5–10× between profiles, so there is no single "N levels is fine" number:

  | nesting form           | release: ok / first abort | debug: ok / first abort |
  | ---------------------- | ------------------------- | ----------------------- |
  | inline table           | 5,000 / 10,000            | 1,000 / 2,000           |
  | array                  | 10,000 / 20,000           | 2,000 / 5,000           |
  | dotted key             | 50,000 / 100,000          | 5,000 / 10,000          |
  | table header           | 50,000 / 100,000          | 5,000 / 10,000          |
  | array-of-tables header | 50,000 / 100,000          | 5,000 / 10,000          |

  (The spread is consistent with inline tables and arrays costing a parser
  frame per level *on top of* the drop frame every form pays, which is also
  why those two are the forms `RecursionGuard` watches — but that reading was
  not traced through `document.rs`'s key-path insertion, only inferred from
  these numbers.)
- *Composition ceiling*, the one genuinely surprising result: the two limits
  **multiply** rather than add, because a dotted key can sit at every level
  of nesting. The deepest file is three parts, each at its own cap: an
  80-segment **array-of-tables** header (81 levels — its last segment is an
  array holding a table, one more than a plain `[a.a…]` header spends), then
  80 nested inline tables each keyed by a fresh 80-segment dotted key (80
  levels apiece), then an 80-segment dotted key for the leaf (79 more).
  13,448 bytes, 6,561 nested tables/arrays counting the document root (the
  scalar at the bottom is the 6,562nd node on that path) — and nothing a
  config file can express goes deeper. Two shallower spellings measured for
  contrast, both of which parse all the way through: a plain `[a.a…]` header
  with the same body is 6,560 (13,446 bytes), and with a plain `a = 1` leaf
  instead of a dotted one it is 6,481 (13,288 bytes).
- *Where the stack goes*: not the parse — the recursive **drop** of the
  parsed `DeTable`. Minimum surviving thread stack for that 13,448-byte worst
  case at flexwm's `FileConfig` target, Linux, binary-searched in 4 KiB (one
  page) steps with one process per trial:

  | | release | debug |
  | --- | --- | --- |
  | parse only (`DeTable::parse` + `mem::forget`) | < 134 KiB | 476 KiB |
  | parse + drop (`toml::from_str::<FileConfig>`) | 932 KiB | 6,680 KiB |
  | the committed test's whole body (`load_from`) | 1,140 KiB¹ | 6,684 KiB |
  | a `toml::Table` target instead | 7,288 KiB² | 32,100 KiB |

  ¹ `cargo` ignores `profile.release.panic` for test targets, so a release
  *test* binary unwinds where the shipped binary aborts, and on the two
  `FileConfig` rows its landing pads cost ~200 KiB more: the same parse+drop
  measures 932 KiB with `panic = "abort"` (what flexwm ships) and
  1,136–1,140 KiB with unwind (what `cargo test --release` builds). The old
  1,135 KiB figure here was an unwind measurement; both are recorded now so a
  re-measurement matches whichever was run.

  ² The `toml::Table` row moves the *other* way between panic strategies:
  7,288 KiB with `panic = "abort"`, 6,468 KiB with unwind (measured both in
  the scratch probe and in flexwm's own release test binary). 7,288 is the
  headline because it is both the shipped profile and the conservative
  number, but a reviewer re-measuring via `cargo test --release` should
  expect 6,468, not something above 7,288.

  Release "parse only" has no point value on Linux via a thread-stack sweep:
  glibc floors a thread stack at 137,120 bytes here and the parse survives
  that floor, so all that can be said is "< 134 KiB". macOS *is* measurable
  there (84 KiB) — the old "79 KiB" attributed to Linux was a macOS number.
  Of the five rows with a Linux point value to compare against, macOS tracks
  Linux 16–336 KiB lower (0.3–5%) on all five, never higher — as parse+drop
  / `toml::Table` / parse-only: release 916 / 7,268 / 84 KiB, debug 6,660 /
  31,764 / 452 KiB.

  flexwm parses on the main thread (`main` → `compositor::run` →
  `config::load`), so it has 8 MiB: over 7 MiB spare in release, 8,192 −
  6,684 = 1,508 KiB (~1.5 MiB) spare in debug. The `toml::Table` row is the
  one that nearly runs out — it fits in release with 904 KiB to spare, and
  does not fit at all in debug.
- *End to end* at `3cb51fc`, binaries rebuilt from that tree (this PR moved
  only doc comments and the test's worst-case bytes, so the production code
  here is unchanged from `e6a983a`'s — but the file under test is not, hence
  the re-run), with the real debug binary on the dev VM
  (`/var/cargo-target/debug/flexwm --headless --config …`): all three 13 KB
  worst-case files (under an unknown key, under `[binds]`, under `[layout]`;
  13,448 / 13,452 / 13,453 bytes) plus the 4 MB 1,000,000-level file started
  normally, logged `ERROR … could not parse config file; using defaults`, had
  no `stack overflow` line, answered `msg version` over IPC, and shut down
  cleanly on `SIGTERM` (exit 143). A legitimate config alongside them loaded
  normally.
- *Reduced stack rlimit*, same `3cb51fc` binaries and the 13,448-byte file: a
  debug build under `ulimit -s 2048` prints `thread 'main' has overflowed its
  stack` / `fatal runtime error: stack overflow, aborting` and dies with
  SIGABRT (exit 134) — the 6,684 KiB it needs does not fit in 2 MiB. The
  release build is fine there (932 KiB), and both are fine at the default
  8192. This is why the resolution above is scoped to the default rlimit.
- *The committed test's own margin*, measured inside flexwm's debug test
  binary by temporarily making its thread size settable: aborts at 6 MiB,
  passes at 7 MiB — consistent with the 6,684 KiB above, and why the test
  runs on an 8 MiB thread (what production gets) rather than `cargo test`'s
  2 MiB.

**Recorded rather than fixed**, in `parse_or_defaults`' doc, because it is
invisible at the site that would break it: moving config parsing to a
spawned thread would get the Rust default 2 MiB and abort a debug build on
that 13 KB file; launching under a reduced `ulimit -s` does the same; and
deserializing the same bytes into a `toml::Table` (a passthrough config
section, say) descends the whole tree instead of stopping at the first
unknown key — 7,288 KiB release, which still fits on the main thread but
leaves under 1 MiB instead of over 7, and 32,100 KiB debug, which does not
fit at all.

**Caveat:** every measurement above is aarch64 (macOS + the dev VM); nothing
was run on x86_64, and this repo has no CI, so the debug-build margin is
only exercised where someone runs `cargo test`.
