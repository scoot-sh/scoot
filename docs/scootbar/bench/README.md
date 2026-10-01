# scootbar's benchmark runs

Each milestone's run of [`scripts/scootbar-bench`](../../../scripts/scootbar-bench/bench.py),
kept as it came out, so the next milestone has a baseline for the resource
ratchet's no-regression rule ([lightest](../backlog/lightest.md)):

```sh
python3 scripts/scootbar-bench/bench.py report docs/scootbar/bench/m1-clock
python3 scripts/scootbar-bench/bench.py compare /tmp/new-run docs/scootbar/bench/m1-clock
```

`report` is the ratchet's rule 2 (no competitor ahead) and `compare` its
rule 1 (no regression against the baseline); each exits 1 when its rule
fails.

A directory per run: `meta.json` (the machine, commit, versions, store
paths, settings and the static rows), `runs.jsonl` (every raw run) and
`table.md` (what `report` printed); the M3 runs also keep `run.log`, the
console output of the run (`m1-clock` predates it). Never overwritten; a new milestone adds
its own. The method is in [testing.md](../testing.md#benchmark).

| Run | Scope | Commit | Where |
|---|---|---|---|
| [`m1-clock`](m1-clock/table.md) | the clock | `3801c12` | [testing-and-ci](../backlog/resolved/testing-and-ci-done.md#evidence): a Claude Code cloud sandbox VM (4-vCPU, not hardware the maintainer owns), headless scoot (debug build) and sway 1.12 |
| [`m3-asahi-m1-baseline-clock`](m3-asahi-m1-baseline-clock/table.md) | the clock | M1 `3801c12` rebuilt | the Asahi M2 (8 CPUs aarch64, Linux 7.1.13), release scootbar, headless scoot (release) and sway 1.12; scootbar only; the like-for-like baseline for M3; `meta.json`'s `commit` is the harness tree (`1beece52`, since amended to `7e8a8b5`, same tree) and `bars.scootbar.source` the tree the binary was built from |
| [`m3-asahi-m1-baseline-clock-rerun`](m3-asahi-m1-baseline-clock-rerun/table.md) | the clock | M1 `3801c12` rebuilt | the same, second run, for the noise |
| [`m3-asahi-clock`](m3-asahi-clock/table.md) | the clock | M3 `3211551` | the same machine and harness; scootbar only |
| [`m3-asahi-clock-rerun`](m3-asahi-clock-rerun/table.md) | the clock | M3 `3211551` | the same, second run |
| [`m3-asahi-clock-workspaces`](m3-asahi-clock-workspaces/table.md) | clock and workspaces | M3 `3211551` | the same machine; scootbar, yambar (sway only) and Waybar, on scoot and sway |
| [`m3-asahi-clock-bindfix`](m3-asahi-clock-bindfix/table.md) | the clock | `cbbeffd` (M3 plus the unplaced-workspaces bind fix) | the same machine and harness; scootbar only; a **single run**; release `scootbar` built from that commit, headless `scoot`/`scootctl` release built from the same tree (source identical to `main` at `90a96ca`, no `crates/scoot` change) |
