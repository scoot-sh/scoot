# scootui extraction: scootbar and scootbg before and after (2026-11-05)

Evidence for the "Done when" audit of
[extract-scootui](../../../docs/scootbar/backlog/resolved/extract-scootui-done.md).
Measurement only; no product code changed.

Machine: the dev VM (aarch64, 6 CPUs, kernel 6.18.50, cgroup v2), shared but idle
(load average under 0.1 between runs). Not the Asahi M2 of the `m4-*` and
`2026-10-08-*` runs, so these cannot be compared with those.

| Tree | Ref | Built |
|---|---|---|
| A (pre) | `7bc849614` (2026-10-18, the claim commit before `extract-draw`) | `git archive` to `/var/tmp/eb-pre`, `cargo build --release -j3 -p scootbar -p scootbg` |
| B (head) | `73569ff79` (v0.7.3) | `/var/tmp/eb-head`, `-p scoot -p scootbar -p scootbg`; this `scoot` is the compositor for every run |

Each tree has its own `CARGO_TARGET_DIR`, `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0`. The harnesses are B's (the same files for both trees).
Run as root through `nix shell nixpkgs#python3 nixpkgs#binutils nixpkgs#imagemagick`
inside `nix develop path:/var/tmp/eb-head`.

```sh
# scootbar: order pre1, head1, head2, pre2 (logs: scootbar/run-and-compare.log)
python3 scripts/scootbar-bench/bench.py run --out OUT --compositors scoot --bars scootbar \
  --scootbar TREE-target/release/scootbar --scootbar-source TREE --scoot HEAD-target/release/scoot
python3 scripts/scootbar-bench/bench.py compare NOW BASELINE
# scootbg: order pre1, head1, pre2 (logs: scootbg/run-and-compare.log)
python3 scripts/scootbg-bench/bench.py run --out OUT --only startup,restore,idle \
  --daemons scootbg --rounds 5 --images /var/tmp/eb-images \
  --scoot HEAD-target/release/scoot --scootbg TREE-target/release/scootbg
python3 scripts/scootbg-bench/bench.py compare NOW BASELINE
```

The benchmark JPEG was generated with `magick -limit thread 3 ...` (ImageMagick
7.1.2-34 here) and has the recorded sha256 `301279ff...27a0`. The first scootbg
attempt, with the default thread count, was refused by the harness for that mismatch
and produced no results.

`pre1`, `head1`, ... are the runs in order; `meta.json`, `runs.jsonl` and `table.md` as in the
other bench directories. The scootbg `compare` output in `run-and-compare.log`
is cut at its top by `tail -40`; every row is in the `table.md` files.
Release stripped sizes, scootbg: 1,905,576 B in A and B (the binaries differ in content).
