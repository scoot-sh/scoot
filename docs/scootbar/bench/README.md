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
`table.md` (what `report` printed). Never overwritten; a new milestone adds
its own. The method is in [testing.md](../testing.md#benchmark).

| Run | Scope | Commit | Where |
|---|---|---|---|
| [`m1-clock`](m1-clock/table.md) | the clock | `3801c12` | [testing-and-ci](../backlog/resolved/testing-and-ci-done.md#evidence): a 4-vCPU container, headless scoot (debug build) and sway 1.12 |
