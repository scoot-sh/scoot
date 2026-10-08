# Restore-at-idle re-run for PR #529 and PR #531 (2026-10-08)

Measurement only. No product code changed, nothing waived.

- **Verdict PR #529** (animated images): no restore regression at idle. scootbg Restore CPU/latency for the JPEG are `same` as base on all rows (0 regressions by the harness `compare`).
- **Verdict PR #531** (wallpaper per workspace): no restore regression at idle. scootbg Restore CPU/latency for the JPEG are `same` as base on all rows (0 regressions by the harness `compare`).

The awww lead the earlier run flagged is competitor-side, not an scootbg regression: at idle it exists in the base run too (base: awww 264 vs scootbg 289 ms, margin 23.6, `beaten`; A and B show the same ~20-27 ms gap, `tie` there only because their spreads widen the margin). scootbg's own medians are 285-289 ms across all three trees.

## What ran

Three trees, shipped to the M2 as `git archive` (no `.git`; SHAs below are the refs archived):

| Tree | Ref | SHA |
|---|---|---|
| base | `origin/main` as of the runs | `b357a4e7219c5d23a2b8b456369cda863cab9630` |
| A (PR #529) | `origin/feat/scootbg-animated-images` | `a90da2ad19fa2dceb99d6b3511c4c9a9bbcbeaa7` |
| B (PR #531) | `origin/feat/scootbg-per-workspace` | `672134b2d752331c0b9d9afbaf4c03ea53043010` |

Base is 7 commits past the earlier run's `ddccc0fea` (6 in `5979b0f51` plus this run's docs-only `b357a4e72`); `git log ddccc0fea..HEAD -- crates/scootbg crates/scootbg-mem crates/scoot scripts/scootbg-bench` is empty, so no measured code changed. Harness `bench.py` is byte-identical in all three trees (sha256 `00cf373a...`). Neither PR touches `scripts/scootbg-bench/`.

Machine: the same Asahi M2 (`Linux 7.1.13 aarch64`), headless scoot built from each tree. Each tree: `cargo build --release -p scoot -p scootbg` inside its own `nix develop` (rustc 1.97.1), each with its own `CARGO_TARGET_DIR` (deleted after the two binaries were copied out):

| Tree | `scoot` sha256 (short) | `scootbg` sha256 (short) |
|---|---|---|
| base | `36c03f64…` | `6c920f5c…` |
| A | `64f6a327…` | `421d6498…` |
| B | `88bcfd54…` | `1e83aeed…` |

Bench ran as root (cgroups) via `sudo nix shell --inputs-from base nixpkgs#python3 nixpkgs#binutils nixpkgs#imagemagick` (Python 3.14.7, GNU strip 2.46, ImageMagick 7.1.2-29, the flake's pin), outer `XDG_RUNTIME_DIR` under the run's own `~/fx/bgrestore-*/`, sessions in their own `/tmp/sbb*` (removed after). The image is the recorded 6000x4000 JPEG, byte for byte (sha256 `301279ff...27a0`, 8,851,735 B), generated once with `magick -limit thread 3 ...` (M2 default 8 threads gives different bytes) and shared via `--images`.

Per tree-round (60 total: 10 global rounds x 3 trees in p1 order base,A,B, then 10 in p2 order B,A,base):

```sh
python3 scripts/scootbg-bench/bench.py run --out out-<p>-<tree>-r<RR> --rounds 1 \
    --only restore --daemons scootbg,awww \
    --scoot bins-<tree>/scoot --scootbg bins-<tree>/scootbg --images images
```

Only the Restore scenario (plus the automatic `check`; no Set/Idle/Startup). `--only restore` still weighs the static Size/Disk rows from the binaries (no `--scootbg-store`, so Disk is binary+closure as weighed, not the nix package; irrelevant to the verdict). Before **every** tree-round the 1-min load average was checked to be below 0.30 (33 waits logged when other load appeared); every record carries its own `loadavg`. Per-record load median 0.26-0.28, range 0.13-0.60 (the harness's own compositor+daemon, not external load).

Full tables: [`base/table.md`](base/table.md), [`A/table.md`](A/table.md), [`B/table.md`](B/table.md); merged `runs.jsonl` (99-100 records each: 20 tree-rounds of check+restore) and `meta.json` beside each; full `compare` outputs: [`compare-A-vs-base.md`](compare-A-vs-base.md), [`compare-B-vs-base.md`](compare-B-vs-base.md). Each combined dir's `meta.json` lists its 20 source out dirs and notes the merge.

## Numbers (Restore JPEG on 1x3840x2160, 20 samples per cell except base awww n=19)

| Row | base | A (#529) | B (#531) | A vs base | B vs base |
|---|---|---|---|---|---|
| Restore: to the JPEG on screen (ms) | 323 [318-328] | 323 [309-335] | 323 [310-330] | same (margin 35.7) | same (margin 30.2) |
| Restore: CPU, JPEG (ms) | 289 [280-291] | 285 [281-291] | 288 [280-293] | same (margin 21.8) | same (margin 24.3) |
| Restore: to a color on screen (ms) | 29.0 [27.8-37.9] | 28.5 [19.8-37.6] | 29.3 [27.4-37.0] | same | same |
| Restore: CPU, color (ms) | 0.6 [0.5-1.3] | 0.5 [0.4-1.2] | 0.6 [0.5-0.7] | same | same |

awww against scootbg in the same runs (gate rule, Restore CPU JPEG): base awww 264 [255-267] vs scootbg 289, margin 23.6, `beaten`; A awww 265 [255-267] vs 285, margin 23.1, `tie`; B awww 261 [249-270] vs 288, margin 34.2, `tie`. Latency is a tie in all three (base 331 vs 323 margin 29.5; A 334 vs 323 margin 63.6; B 330 vs 323 margin 99.2). The gaps are the same size in all three trees (~20-27 ms CPU); only the margin moves, because the spread moves. That is competitor/spread noise, not a code change: scootbg's medians differ by 1-4 ms across trees, far inside the 21-24 ms margins.

Order check (p1 vs p2 medians, Restore CPU JPEG): base 289.1 / 289.1; B 287.8 / 288.5; A 289.0 / 282.0. A's 7 ms pass difference is inside the margin. No order effect changes a verdict.

Notes, all checked to be non-load-bearing: awww's `check` flaked once in 60 invocations (`out-p1-base-r05`: client failed, broken pipe, the known awww flake; that cell rests on 19 samples, margin still far below the gap logic above). One B awww latency sample hit 390 ms at load 0.28 (its CPU sample that round was the lowest, 248.7 ms: a settle hiccup, not load); without it B's latency spread is 310-341 and the verdict stays `tie`. Both daemons' CPU samples fall in two clusters ~8 ms apart (e.g. scootbg 280-282 vs 288-291), likely frequency scaling; it widens spreads (hence margins) but medians are stable across passes and trees.

## Sizes (from these binaries; not the nix packages)

Stripped `scootbg` Size row: base 1,841,928 B; A and B 1,907,464 B (+65,536, `same` by the margin, matching the earlier run). Disk row without a store weighs binary+closure only and is likewise `same`.

## Raw data

`base/`, `A/`, `B/`: each combined run's `table.md`, `meta.json` (machine, versions, binary SHAs, harness state, the 20 source dirs) and `runs.jsonl` (every raw run with its load average). `compare-A-vs-base.md` and `compare-B-vs-base.md` are the full harness `compare` outputs the verdict columns above come from.
