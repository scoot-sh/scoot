# scootbar's benchmark runs

Each milestone's run of [`scripts/scootbar-bench`](../../../scripts/scootbar-bench/bench.py),
kept as it came out, so the next milestone has a baseline for the resource
ratchet's no-regression rule ([lightest](../../../docs/scootbar/backlog/lightest.md)):

```sh
python3 scripts/scootbar-bench/bench.py report dev/benches/scootbar/m1-clock
python3 scripts/scootbar-bench/bench.py compare /tmp/new-run dev/benches/scootbar/m1-clock
```

`report` is the ratchet's rule 2 (no competitor ahead) and `compare` its
rule 1 (no regression against the baseline); each exits 1 when its rule
fails.

A directory per run: `meta.json` (the machine, commit, versions, store
paths, settings and the static rows), `runs.jsonl` (every raw run) and
`table.md` (what `report` printed); the M3 runs also keep `run.log`, the
console output of the run (`m1-clock` predates it). Never overwritten; a new milestone adds
its own. The method is in [testing.md](../../research/scootbar-testing.md#benchmark).

| Run | Scope | Commit | Where |
|---|---|---|---|
| [`m1-clock`](m1-clock/table.md) | the clock | `3801c12` | [testing-and-ci](../../../docs/scootbar/backlog/resolved/testing-and-ci-done.md#evidence): a Claude Code cloud sandbox VM (4-vCPU, not hardware the maintainer owns), headless scoot (debug build) and sway 1.12 |
| [`m3-asahi-m1-baseline-clock`](m3-asahi-m1-baseline-clock/table.md) | the clock | M1 `3801c12` rebuilt | the Asahi M2 (8 CPUs aarch64, Linux 7.1.13), release scootbar, headless scoot (release) and sway 1.12; scootbar only; the like-for-like baseline for M3; `meta.json`'s `commit` is the harness tree (`1beece52`, since amended to `7e8a8b5`, same tree) and `bars.scootbar.source` the tree the binary was built from |
| [`m3-asahi-m1-baseline-clock-rerun`](m3-asahi-m1-baseline-clock-rerun/table.md) | the clock | M1 `3801c12` rebuilt | the same, second run, for the noise |
| [`m3-asahi-clock`](m3-asahi-clock/table.md) | the clock | M3 `3211551` | the same machine and harness; scootbar only |
| [`m3-asahi-clock-rerun`](m3-asahi-clock-rerun/table.md) | the clock | M3 `3211551` | the same, second run |
| [`m3-asahi-clock-workspaces`](m3-asahi-clock-workspaces/table.md) | clock and workspaces | M3 `3211551` | the same machine; scootbar, yambar (sway only) and Waybar, on scoot and sway |
| [`m3-asahi-clock-bindfix`](m3-asahi-clock-bindfix/table.md) | the clock | `cbbeffd` (M3 plus the unplaced-workspaces bind fix) | the same machine and harness; scootbar only; a **single run**; release `scootbar` built from that commit, headless `scoot`/`scootctl` release built from the same tree (source identical to `main` at `90a96ca`, no `crates/scoot` change) |
| [`m3-asahi-clock-workspaces-all`](m3-asahi-clock-workspaces-all/table.md) | clock and workspaces | harness `80241e5f` (scootbar: `main` `90a96ca0`, M3's code) | the same machine; scootbar, yambar (sway only), Waybar, plus **ironbar** (sway only) and **ashell** as informational columns the gate does not count, on scoot and sway; `main`'s bind fix (#363) is not in it |
| [`m4-asahi-clock-main`](m4-asahi-clock-main/table.md), [`-rerun`](m4-asahi-clock-main-rerun/table.md) | the clock | `main` `7a1f9030a` | the same machine; scootbar only, on scoot; **harness from `main` `7a1f9030a`** (scootbar's config and measuring code unchanged since `cbbeffd`), one release `scoot`/`scootctl` from the same tree for every `m4-*` run; the M4 baseline, which reproduces `m3-asahi-clock-bindfix` (`compare` exit 0) |
| [`m4-asahi-clock-both-main`](m4-asahi-clock-both-main/table.md), [`-both-pr367`](m4-asahi-clock-both-pr367/table.md), [`-both-pr367-opt-s`](m4-asahi-clock-both-pr367-opt-s/table.md) | the clock | `main`, #367 and the experiment as above | **round three: scoot and sway** (`--compositors scoot,sway`, about 13 minutes each), one run each |
| [`m4-asahi-clock-pr364`](m4-asahi-clock-pr364/table.md) | the clock | #364 `47b413a99` (pointer input) | the same; one run |
| [`m4-asahi-clock-pr366`](m4-asahi-clock-pr366/table.md) | the clock | #366 `cdb9936bd` (`button`, `push`, `exec`) | the same; one run |
| [`m4-asahi-clock-pr367`](m4-asahi-clock-pr367/table.md), [`-rerun`](m4-asahi-clock-pr367-rerun/table.md) | the clock | #367 `b6dee9710` (the agent interface) | the same; two runs |
| [`m4-asahi-clock-pr367-opt-s`](m4-asahi-clock-pr367-opt-s/table.md), [`-rerun`](m4-asahi-clock-pr367-opt-s-rerun/table.md) | the clock | #367 plus `[profile.release.package.scootbar] opt-level = "s"` | **an experiment, in no PR**: a local commit on the Asahi box (`854f3394`, since removed) (the five added lines of `Cargo.toml`, quoted in the [README](../../../docs/scootbar/README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2), on `b6dee9710`); two runs |
| [`m4-asahi-final-main`](m4-asahi-final-main/table.md), [`-pr367`](m4-asahi-final-pr367/table.md), [`-main-rerun`](m4-asahi-final-main-rerun/table.md) | the clock | `main` `98c4b7a32`, #367's fixed tip `d0c4f35d1`, `main` again (A-B-A) | **the final stack**: scoot and sway, one run each; **`main` has moved** (it carries #370, which the #367 tip lacks) and **the pinned `scoot` is a rebuilt binary** (sha256 `25e6a225a07d...`, not the earlier runs' `4e105065f5b2...`); harness files sha256-identical to the earlier `m4-*` runs; the first run started warm (load 0.44); `python3` from `/nix/store/3fl7bdkdxk2k4nssy1d1161isbzn6bsr-python3-3.14.7`; every `compare` and pooled verdict in [`m4-asahi-final-compares.md`](m4-asahi-final-compares.md), the table and what changed in the [README](../../../docs/scootbar/README.md#m4-final-stack-the-fix-round-re-measured) |
| [`m6-tray-vm`](m6-tray-vm/README.md) | the tray and the D-Bus client | `9e67c9fbd` | the dev VM (6 CPUs aarch64, shared), release scootbar and `main` `f4c93a63a`, headless scoot, a private `dbus-daemon`; **not** a `scripts/scootbar-bench` run: shell scripts, real D-Bus items on `jeepney`, one run per row |

The `m4-*` runs are the M4 stack against M3 post-fix, in three rounds: the first two (scoot only, the second in reverse order) and a third, `-both-`, with scoot **and sway** for `main`, #367 and the experiment, as the post-fix baseline ran (#364 and #366 have no sway run); `meta.json`'s `commit` is the harness tree (`7a1f9030a`) for all of them and `bars.scootbar.source` the tree each binary was built from. Their `run.log` is the console output, whose first line is a `sudo` warning about `$HOME`. Every `compare` and the pooled verdicts are in [`m4-asahi-compares.md`](m4-asahi-compares.md); the tables are in the [README](../../../docs/scootbar/README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2).

`m3-asahi-clock-workspaces-all`'s `meta.json` names the harness commit `80241e5f`, which a rebase onto main (after the bind fix) rewrote to `d13d28d3d`; the `scripts/` tree is identical.
