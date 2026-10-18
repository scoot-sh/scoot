# Text shaping spike: `rustybuzz` + `unicode-bidi`, measured

Measured 2026-10-18 for
[ticket](../../../docs/scootbar/backlog/scootui-text-shaping-spike.md), from
[shell-apps.md §Text](../design/shell-apps.md#text-shaping-wrapping-emoji).
Three throwaway binaries sharing one harness, none of them in the
workspace: `baseline` (`ab_glyph` only, today's `text.rs` path), `bidionly`
(`unicode-bidi` visual runs, one glyph per char), `shaped` (`unicode-bidi`
plus `rustybuzz`). Sized and timed on the dev VM (aarch64); timing and
renders repeated on the Mac. The spike crates are removed after; this file
is the record, with the side-by-side figures in
[`text-shaping/`](text-shaping/).

**Outcome: NO-GO. Shaping does not ship in scootnotify's default build.**
The per-body time passes the gate with margin, but the `.text` cost is
343.4 KiB against a gate of about 250 KiB, and the dependency set is
already minimal (`rustybuzz` default features are `std` only). v1 ships
without shaping, and the scripts that draw wrong are listed below. A
`unicode-bidi`-only build (+23.0 KiB) fixes line order but not letterforms;
adopting it is a separate ticket, not this gate.

## What was run

- One harness: a grayscale canvas, an 8-line fixed corpus (Latin,
  Cyrillic, CJK, Arabic, Hebrew, Devanagari, emoji with a monochrome emoji
  fallback, one mixed bidi line), a deterministic exactly-500-byte
  Arabic-heavy body (313 chars) and a Latin 500-byte body for context.
- `baseline`: per-char resolve through the font chain, pen advanced by the
  advance, logical order. Timed: measuring the 500-byte body.
- `bidionly`: `BidiInfo` visual runs, RTL runs reversed, one glyph per
  char. Timed: reordering the body.
- `shaped`: visual runs shaped with `rustybuzz` (no OpenType feature
  overrides), drawn in visual order with shaped advances and offsets,
  rasterized through `ab_glyph`. Runs are split by chain-font coverage, the
  spike's stand-in for v1 fallback. Timed: bidi plus `rustybuzz` over the
  body, faces built once up front as a real caller would; glyph
  rasterization excluded (cached on first draw on either path).
- Release profile matching the workspace (`lto = "fat"`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`, `opt-level = 3`).
- Fonts: DejaVu Sans 2.37, Noto Sans SC / Arabic / Hebrew / Devanagari /
  Emoji (variable, default instance, google/fonts as of 2026-10-18). The
  SC font has no Hangul: the Korean boxes in every render are font
  coverage, identical on all three paths, not a shaping finding.

## Results

Gate: under about 250 KiB of `.text`, under 1 ms per body.

| | `baseline` | `bidionly` | `shaped` |
|---|---|---|---|
| ELF `.text` (`size -A`) | 350,608 B | 374,160 B | 702,260 B |
| Added `.text` | — | +23,552 B (23.0 KiB) | +351,652 B (**343.4 KiB**) |
| ELF `.rodata` | 36,411 B | 56,051 B | 208,824 B (+168.4 KiB tables) |
| Stripped binary | 529,080 B | 529,080 B | 1,184,440 B (+640.0 KiB) |
| `VmHWM` after full-corpus render | 46,448 kB | 46,320 kB | 47,728 kB (+1.3 MB) |
| 500-byte mixed body, mean / median / min | 16.8 / 15.1 / 11.0 µs | 31.9 / 32.8 / 17.3 µs | **212.9 / 196.4 / 130.0 µs** |
| 500-byte Latin body, mean / median / min | — | 22.3 / 23.0 / 13.3 µs | 158.0 / 171.6 / 71.0 µs |

2000 iterations after 200 warmup, dev VM (Apple-silicon aarch64, 6 vCPUs,
rustc 1.97.1). Earlier full runs agreed (mixed-body mean 200–233 µs): the
gate passes at roughly a quarter of the 1 ms budget on every run. On the Mac (rustc 1.98.1, same fonts): mixed-body
shape 68.8 / 65.2 / 59.9 µs, Mach-O `__TEXT` +512.0 KiB over baseline.
Timing passes on both; size fails on both.

`ttf-parser` resolves to a single 0.25.1 in the spike lock, shared between
`rustybuzz` 0.20.1 and `ab_glyph`'s `owned_ttf_parser` 0.25.1, as the
ticket asked to confirm. No version of the plan's size knobs changes the
verdict: the feature set is already `std`-only, and the remaining bulk is
HarfBuzz shaping code plus 168 KiB of Unicode tables, not optional data.

## Correctness

Each figure stacks `baseline` / `bidionly` / `shaped` for one corpus line.

| Script | Without shaping (v1) | Figure |
|---|---|---|
| Latin, Cyrillic | fine (no kerning, visible but acceptable) | `triple-latin.png`, `triple-cyrillic.png` |
| CJK | fine; no shaping involved, identical on all paths | `triple-cjk.png` |
| Arabic | **wrong**: isolated letterforms in logical order | `triple-arabic.png` |
| Hebrew | **wrong**: logical (LTR) order | `triple-hebrew.png` |
| Devanagari | **wrong**: no conjuncts, unformed matras | `triple-devanagari.png` |
| Emoji (monochrome fallback) | fine: plain glyph coverage, no shaping needed | `triple-emoji.png` |
| Mixed bidi line | **wrong** order and forms; `bidionly` fixes order only | `triple-mixed.png` |

Full corpus in [`corpus-triple.png`](text-shaping/corpus-triple.png).
Color emoji stays out of scope on every path (v1 non-goal).

## Licenses added

`cargo tree` delta of `shaped` over `baseline`, checked against
`deny.toml` (allow: MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0,
Zlib, Unlicense, 0BSD). Every added crate passes on at least one side:

| Crate | Version | License | Verdict |
|---|---|---|---|
| `rustybuzz` | 0.20.1 | MIT | allow |
| `unicode-bidi` | 0.3.18 | MIT OR Apache-2.0 | allow |
| `unicode-bidi-mirroring` | 0.4.0 | MIT/Apache-2.0 | allow |
| `unicode-ccc` | 0.4.0 | MIT/Apache-2.0 | allow |
| `unicode-properties` | 0.1.4 | MIT/Apache-2.0 | allow |
| `unicode-script` | 0.5.8 | MIT OR Apache-2.0 | allow |
| `smallvec` | 1.16.3 | MIT OR Apache-2.0 | allow |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | allow |
| `core_maths` | 0.1.1 | MIT | allow |
| `bitflags` | 2.13.2 | MIT/Apache-2.0 | allow |
| `libm` | 0.2.16 | MIT | allow |
| `log` | 0.4.34 | MIT OR Apache-2.0 | allow |

(`ab_glyph` 0.2.32 and `ttf-parser` 0.25.1 are the existing stack, not
added.) No advisory check was run: the crates never enter the workspace,
so `cargo deny` has nothing new to gate on; the table above is the
ticket's license evidence.

## Evidence

Binaries built on the dev VM over ssh (`ssh -p 2222 dev@localhost`),
`CARGO_TARGET_DIR=~/w8-shaping-target` (own dir, removed after), sources
shipped by tar (sha256
`ed1dfdc22abec5625f9e98ca7a74af33ed5bfefcc6c36ed8f7974302edef94f2`).
Workspace state for every number on this page:
`64e4a75ebdf4f74e4b652d0c6b0660ea6d126c90` (origin/main at the time;
spike code itself never entered the tree). Fonts in `~/w8-fonts` on the
VM (kept for re-runs); target dir and tarball removed.

Raw final pass (this is the run the tables above quote):

```
baseline: body_bytes=500 body_chars=313
baseline_measure_mean_ns=16831 median=15125 min=11000 iters=2000
baseline_vmhwm_kb=46448 baseline_vmrss_kb=45704
bidionly_mixed_mean_ns=31908 median=32833 min=17292
bidionly_latin_mean_ns=22331 median=22958 min=13333
bidionly_vmhwm_kb=46320
shaped_shape_mean_ns=212899 median=196416 min=130000 iters=2000
shaped_latin_mean_ns=158038 median=171625 min=71042
shaped_vmhwm_kb=47728 shaped_vmrss_kb=46892
```

`.text`/stripped rows from `size -A` and `stat` on the same binaries;
`VmHWM`/`VmRSS` from `/proc/self/status` at exit after the full-corpus
render. Not measured: color emoji, line breaking, per-cluster fallback
beyond run splitting, `ShapePlan` caching (would only improve the passing
time row).
