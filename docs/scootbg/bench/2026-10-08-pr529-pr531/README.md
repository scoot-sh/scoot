# scootbg release benchmark re-run for PR #529 and PR #531 (2026-10-08)

Measurement only. No product code changed, nothing waived: the maintainer rules.

- **Verdict PR #529** (animated images, stage 1): two rows beyond the margin against base, both idle PSS above the floor with an image (1x1080p: 2.35 to 2.49 MiB; 2x4K: 2.25 to 2.38 MiB), both in the class the maintainer waived on 2026-09-28; every other row is the same against base.
- **Verdict PR #531** (wallpaper per workspace): eight rows beyond the margin against base, all idle PSS (six gated: +0.13 to +0.17 MiB; plus the two ungated raw-PSS color rows); four of the six gated rows are in the waived above-the-floor class and the other two are total-with-floor color rows where scootbg still beats every competitor by 7 MiB or more; every other row is the same against base.

## What ran

Three trees, each shipped to the M2 as a `git archive` of its ref (no `.git`, so each run's `meta.json` records an empty harness commit; the SHAs below are the refs archived, verified by file hash after shipping):

| Tree | Ref | SHA |
|---|---|---|
| base | `origin/main` as of the runs | `ddccc0feaa8eda0b88a65f23df0c0f15d3b3ec55` |
| A (PR #529) | `origin/feat/scootbg-animated-images` | `a90da2ad19fa2dceb99d6b3511c4c9a9bbcbeaa7` |
| B (PR #531) | `origin/feat/scootbg-per-workspace` | `672134b2d752331c0b9d9afbaf4c03ea53043010` |

Neither PR touches `scripts/scootbg-bench/` (verified by diff), so all three runs used the same harness. Since the runs, `origin/main` has moved 6 commits (`ddccc0fea..5979b0f51`); all six touch scootbar, docs, site or packaging only, none touch `crates/scootbg*`, `crates/scoot` or the harness, so no measured row is affected.

Machine: the Asahi M2 (`Linux 7.1.13 aarch64`, 8 cores, 7.7 GB RAM, NixOS), headless scoot built from each tree. The published runs were on a 4-vCPU Intel Xeon container, so absolute numbers against them are indicative only; the rigorous verdicts are the same-machine A-vs-base and B-vs-base `compare` runs below, by the documented margin (larger of 5% of the reference median, the two sides' combined spread, and the unit floor).

Each tree: `cargo build --release -p scoot -p scootbg` inside the tree's own `nix develop` (pinned rustc 1.97.1), each with its own `CARGO_TARGET_DIR` (deleted after the two binaries were copied out), plus `nix build .#scootbg` for the Disk row (base's package substituted from the project's cachix, i.e. CI's build of byte-identical sources; A and B built locally). Then, per tree:

```sh
python3 scripts/scootbg-bench/bench.py run --out out-<base|A|B> --rounds 5 \
    --scoot bins-<t>/scoot --scootbg bins-<t>/scootbg \
    --scootbg-store "$(readlink -f pkg-<t>)" --images images
```

Same flags as the published runs (5 rounds, 60 s idle, 60 s window), run as root (the harness needs to create cgroups). The three runs went sequentially, base then A then B, not round-robin: the box was otherwise idle (load ~0.00-0.09 at every launch; every record carries its own `loadavg`). Full tables: [`base/table.md`](base/table.md), [`A/table.md`](A/table.md), [`B/table.md`](B/table.md); full `compare` outputs: [`compare-A-vs-base.md`](compare-A-vs-base.md), [`compare-B-vs-base.md`](compare-B-vs-base.md).

The image is the recorded 6000x4000 JPEG, byte for byte (sha256 `301279ff...27a0`, 8,851,735 B). One provenance note: ImageMagick's `plasma:fractal` + `+noise Gaussian` output depends on the OpenMP thread count, and the M2 default (8) gives different bytes than the container. A sweep over thread counts with nixpkgs' imagemagick 7.1.2-29 (the flake's pin, same rev and flags as the dev shell) showed `-limit thread 3` reproduces the recorded file exactly; that file was generated once and shared by all three runs (`--images`), so every measurement is on the same bytes.

Competitors at the flake's pinned nixpkgs (same versions as published): awww 0.12.1, swaybg 1.2.2, wbg 1.3.0, wpaperd 1.3.0, and hyprpaper 0.8.4, which **runs here** (the M2 has a DRM device; the published runs record it as did-not-run on both compositors). It beats scootbg nowhere.

Load averages: per-record `loadavg` in `runs.jsonl`. A: 0.00-0.09 at launch, max 1.77 on one early record, idle rows at 0.00-0.01. B: 0.00-0.60 throughout. Base: 2.25 at launch (competitor packages resolving) and 1.0-2.0 through round 4 of the timing rows from load that was not this run's own (the image search and both builds had finished; another agent's builds are the likely source, as briefed). Round 5 and all idle rows ran at 0.00-0.10, and every timing median agrees with the later A/B runs (all `same`), so no verdict depends on the loaded rounds; the numbers are reported as measured.

awww flaked on the M2: its daemon died mid-scenario in some idle rounds (`broken pipe` writing to it, or `none of the requested outputs are valid`), costing samples: base 3 failed rows, A 8, B 5, all awww idle, all at load <= 0.72. The affected cells keep 1-4 of 5 samples (A's awww 2x4K-color idle cells rest on 1 sample); the margins there are still far smaller than the gaps, so no verdict hinges on them. scootbg itself failed nothing in any run.

## Comparison table (gated rows)

`published` is the last published scootbg number for the row: the 2026-09-27 run, except idle rows (the 2026-09-28 idle re-run) and peak PSS (the 2026-09-28 peak runs). `same` / `REGRESSED` are the harness `compare` verdicts against base on this machine.

| Row | published | base | A (#529) | B (#531) | A vs base | B vs base |
|---|---|---|---|---|---|---|
| Idle RSS above the floor, 1x 1080p, image (MiB) | 4.3 [4.2-4.3] | 4.0 [4.0-4.0] | 4.1 [4.1-4.1] | 4.2 [4.2-4.2] | same | same |
| Idle PSS above the floor, 1x 1080p, image (MiB) | 2.3 [2.3-2.4] | 2.3 [2.3-2.4] | 2.5 [2.5-2.5] | 2.5 [2.5-2.5] | REGRESSED | REGRESSED |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1x 1080p, image (MiB) | 10.3 [10.2-10.3] | 10.3 [10.3-10.3] | 10.4 [10.4-10.4] | 10.4 [10.4-10.4] | same | same |
| Idle RSS above the floor, 2x 4K, image (MiB) | 4.4 [4.3-4.5] | 3.9 [3.9-3.9] | 4.0 [4.0-4.0] | 4.1 [4.1-4.1] | same | same |
| Idle PSS above the floor, 2x 4K, image (MiB) | 2.4 [2.4-2.5] | 2.3 [2.2-2.3] | 2.4 [2.4-2.4] | 2.4 [2.4-2.4] | REGRESSED | REGRESSED |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2x 4K, image (MiB) | 34.0 [34.0-34.1] | 33.9 [33.9-33.9] | 34.0 [34.0-34.0] | 34.1 [34.0-34.1] | same | same |
| Idle RSS above the floor, 1x 1080p, color (MiB) | 3.8 [3.8-3.9] | 3.5 | 3.6 [3.5-3.6] | 3.7 [3.7-3.7] | same | same |
| Idle PSS above the floor, 1x 1080p, color (MiB) | 2.0 [2.0-2.1] | 2.1 [2.1-2.1] | 2.2 [2.1-2.2] | 2.2 [2.2-2.3] | same | REGRESSED |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1x 1080p, color (MiB) | 2.0 [2.0-2.1] | 2.1 [2.1-2.1] | 2.2 [2.1-2.2] | 2.2 [2.2-2.3] | same | REGRESSED |
| Idle RSS above the floor, 2x 4K, color (MiB) | 3.8 [3.7-3.8] | 3.5 [3.5-3.5] | 3.6 [3.5-3.6] | 3.7 [3.7-3.7] | same | same |
| Idle PSS above the floor, 2x 4K, color (MiB) | 2.0 [2.0-2.1] | 2.1 [2.1-2.1] | 2.2 [2.1-2.2] | 2.2 [2.2-2.3] | same | REGRESSED |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2x 4K, color (MiB) | 2.1 [2.0-2.1] | 2.1 [2.1-2.1] | 2.2 [2.1-2.2] | 2.2 [2.2-2.3] | same | REGRESSED |
| Idle wakeups in 60 s, 1x 1080p, image () | 0 | 0 | 0 | 0 | same | same |
| Idle wakeups in 60 s, 2x 4K, image () | 0 | 0 | 0 | 0 | same | same |
| Idle wakeups in 60 s, 1x 1080p, color () | 0 | 0 | 0 | 0 | same | same |
| Idle wakeups in 60 s, 2x 4K, color () | 0 | 0 | 0 | 0 | same | same |
| Idle CPU in 60 s, 1x 1080p, image (ms) | 0.0 | 0.0 | 0.0 | 0.0 | same | same |
| Idle CPU in 60 s, 2x 4K, image (ms) | 0.0 | 0.0 | 0.0 | 0.0 | same | same |
| Idle CPU in 60 s, 1x 1080p, color (ms) | 0.0 | 0.0 | 0.0 | 0.0 | same | same |
| Idle CPU in 60 s, 2x 4K, color (ms) | 0.0 | 0.0 | 0.0 | 0.0 | same | same |
| Peak memory (PSS), JPEG at start-up, 1x 4K (MiB) | 84.7 [84.2-85.6] | 85.3 [83.2-85.4] | 84.9 [83.6-85.6] | 84.3 [83.3-84.9] | same | same |
| Peak memory (PSS), live change to the JPEG, 1x 4K (MiB) | 101.3 [100.4-101.4] | 100.5 [99.7-101.2] | 101.5 [101.1-101.7] | 100.6 [99.2-101.2] | same | same |
| Set: latency to the JPEG (ms) | 472 [447-702] | 290 [279-305] | 299 [289-308] | 279 [278-298] | same | same |
| Set: CPU for the JPEG (ms) | 458 [437-683] | 281 [274-297] | 282 [282-286] | 274 [273-282] | same | same |
| Set: latency to a color (ms) | 16.7 [13.9-25.4] | 15.4 [11.8-27.2] | 34.4 [14.3-48.5] | 24.7 [20.2-32.2] | same | same |
| Set: CPU for a color (ms) | 4.3 [3.9-4.4] | 1.7 [1.2-1.9] | 1.8 [1.5-2.3] | 2.0 [1.8-2.2] | same | same |
| Startup: to a color on screen (ms) | 22.2 [20.4-30.0] | 27.2 [22.9-34.2] | 24.4 [21.7-35.3] | 25.3 [17.5-31.5] | same | same |
| Startup: CPU, color (ms) | 5.8 [4.8-7.6] | 4.0 [3.3-4.5] | 2.6 [2.3-3.7] | 2.7 [2.2-3.3] | same | same |
| Startup: to the JPEG on screen (ms) | 469 [465-498] | 310 [300-322] | 305 [301-333] | 304 [297-317] | same | same |
| Startup: CPU, JPEG (ms) | 444 [443-462] | 283 [280-300] | 285 [283-300] | 283 [281-295] | same | same |
| Restore: to the JPEG on screen (ms) | 482 [449-522] | 324 [310-337] | 324 [318-339] | 323 [314-324] | same | same |
| Restore: CPU, JPEG (ms) | 463 [432-484] | 287 [281-295] | 291 [280-297] | 289 [280-291] | same | same |
| Restore: to a color on screen (ms) | 20.0 [11.9-23.8] | 29.2 [28.1-30.5] | 31.7 [27.2-43.7] | 31.5 [27.9-36.8] | same | same |
| Restore: CPU, color (ms) | 2.3 [2.3-3.4] | 0.5 [0.5-0.6] | 0.7 [0.6-0.7] | 0.6 [0.5-0.6] | same | same |
| Size (bytes) | 1,866,680 | 1,841,928 | 1,907,464 | 1,907,464 | same | same |
| Disk (bytes) | 12,172,256 | 34,858,673 | 34,924,209 | 34,989,745 | same | same |

## Rows that move beyond the margin in the wrong direction (against base)

A: 2, by the harness `compare` (2 regression(s)):

- Idle PSS above the floor, 1x1080p, image: 2.35 to 2.49 MiB (+0.14).
- Idle PSS above the floor, 2x4K, image: 2.25 to 2.38 MiB (+0.13).

B: 8, by the harness `compare` (8 regression(s)), six of them gated:

- Idle PSS above the floor, 1x1080p, image: 2.35 to 2.52 MiB (+0.17).
- Idle PSS above the floor, 2x4K, image: 2.25 to 2.41 MiB (+0.16).
- Idle PSS above the floor, 1x1080p, color: 2.09 to 2.25 MiB (+0.16).
- Idle PSS above the floor, 2x4K, color: 2.09 to 2.25 MiB (+0.16).
- Idle total with the floor, 1x1080p, color: 2.09 to 2.25 MiB (+0.16).
- Idle total with the floor, 2x4K, color: 2.09 to 2.25 MiB (+0.16).
- Plus the two ungated raw-PSS color rows (1x1080p and 2x4K, 2.09 to 2.25 MiB), flagged by `compare` but outside the gate.

Of these, the waived class (idle memory above the floor vs awww, and only that class, waived 2026-09-28) covers: both A rows, and four of B's six gated rows (the above-the-floor rows). It does not cover B's two total-with-floor color rows; on those scootbg still beats every competitor that can show a color by 2.2 MiB (swaybg) to 62 MiB (awww) - see the run tables. No waiver is claimed here either way; this is the tabulation.

## Rows where a competitor now beats scootbg beyond the margin that did not before

- Restore CPU for the JPEG vs awww: a tie in the base run (awww 268.4 vs scootbg 287.3 ms), a loss in the A run (259.5 vs 291.1 ms, margin 27.9) and the B run (260.7 vs 289.5 ms, margin 21.0). Competitor-side movement: awww ran 8-9 ms faster in those runs while scootbg is unchanged (`same` against base in all three runs' `compare`). Not an scootbg regression.
- Set CPU for the JPEG vs wpaperd is a loss in all three runs here (base: 237.5 vs 280.8 ms), so it is not new against base; it is new against the published table (where scootbg won 458 vs 844 ms). That is the machine, not the code: the published container had no GPU and slow CPUs, so wpaperd's software-GL path cost 844 ms; on the M2 the same path costs ~237 ms while scootbg's CPU path costs ~274-282 ms. Same verdict method, different hardware.

Otherwise the gate matches the published one on all three runs: the same 9 idle-memory losses to awww (base: 10 losses, A and B: 11, the extras listed above), wins or ties everywhere else, 0 wakeups, hyprpaper beating scootbg nowhere.

## Sizes

Stripped `scootbg` (cargo release build, `strip --strip-all`):

| Tree | File (B) | Delta vs base | `.text` (B) | Delta vs base |
|---|---|---|---|---|
| base | 1,708,832 | - | 1,227,684 | - |
| A (#529) | 1,774,368 | +65,536 | 1,268,772 | +41,088 |
| B (#531) | 1,774,368 | +65,536 | 1,309,892 | +82,208 |

Matches the reviews' figures. The gated Size row (stripped binaries + non-glibc `ldd` closure) moves by the same +65,536 B for both (1,841,928 to 1,907,464), inside the margin. The Disk row likewise (`same` for both); note its absolute level differs from published (34.9 vs 12.2 MB: the aarch64 nix closure is larger) and B's installed package is 65,536 B larger than A's (nix-store binaries: base 1,708,832, A 1,774,368, B 1,839,904) while the stripped Size row is identical for A and B - build-environment padding, inside the margin either way.

## Raw data

`base/`, `A/`, `B/`: each run's `table.md` (the rendered table and gate), `meta.json` (machine, versions, store paths, binary SHAs, harness state, static rows) and `runs.jsonl` (every raw run with its load average). `compare-A-vs-base.md` and `compare-B-vs-base.md` are the full harness `compare` outputs the verdict columns above come from.
