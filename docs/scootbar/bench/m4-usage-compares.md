# M4 usage optimization on the Asahi M2: every compare, as run (2026-10-01)

Run from the repository root with the harness of `main` (`scripts/scootbar-bench`: `bars.py`, `bench.py`, `machine.py`, `measure.py`, `stage.py` and `tables.py` are sha256-identical to the `m4-asahi-final-*` runs'; `redraw.py` and its test are new and not used by the harness). `compare NOW BASELINE`; exit status 1 on a gated regression. **Clock scope**: scoot and sway in every run, the defaults (5 startup rounds, 30 s settle, 300 s idle, 240 switches at 4 Hz), 180 s of cool-down between runs, one pinned release `scoot`/`scootctl` (sha256 `33aa667c...` / `d1ffa1b0...`). The seven runs, in the order they ran: `m4-usage-main` (`main` before M4, `98c4b7a32`), `m4-usage-order` (this PR's build: the stack plus the hot-text order file), `m4-usage-stack` (the stack as it is on `main` at `c9d2cd361`, without the order file), `m4-usage-order-rerun`, `m4-usage-main-rerun`, `m4-usage-order-3`, `m4-usage-main-3` (the last two, so that the CPU row has three runs a side). **Workspaces scope** (`--scope clock-workspaces`, scoot only): `m4-usage-ws-main`, `-ws-order`, `-ws-stack`, `-ws-main-rerun`. The pooled verdicts are at the end. **Cache key:** the harness tree was the uncommitted PR (see each `meta.json`'s `tree_dirty`: `crates/scootbar` and `scripts/` of the PR's `perf` commit `22f9cf279` on `c9d2cd361`); a rebuild from `22f9cf279` is byte-identical to the benchmarked binary (sha256 `860185c2d678e3c8f42d65c445e73401133f36f37d6000da383638f5b0e1d893`).

**How to read the pairings.** `compare` judges one run against one run; the idle CPU row on this box flags in single pairings and not in others (the drift compares at the end show `main` against itself). Every pairing of the order-file build against `main` before M4 exits 1 on the **size row** (+262,176 B, accepted by the maintainer on 2026-10-01, not waived); the pairings that flag anything else are named below and judged by the pooled verdicts.


## The order-file build against `main` before M4 (every pairing)

### `m4-usage-order` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 38.5 [30.8–45.9] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.1 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 16.7 [3.6–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | same |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order` against `m4-usage-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order docs/scootbar/bench/m4-usage-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.4 [30.2–39.6] | 38.5 [30.8–45.9] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [7.6–18.6] | 16.7 [3.6–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | same |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order` against `m4-usage-main-3`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order docs/scootbar/bench/m4-usage-main-3
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.3 [29.6–43.3] | 38.5 [30.8–45.9] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [8.0–17.5] | 16.7 [3.6–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | better |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-rerun` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-rerun docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 36.0 [31.6–46.4] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.1 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 16.6 [6.9–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | same |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.6 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-rerun` against `m4-usage-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-rerun docs/scootbar/bench/m4-usage-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.4 [30.2–39.6] | 36.0 [31.6–46.4] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [7.6–18.6] | 16.6 [6.9–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | same |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.6 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-rerun` against `m4-usage-main-3`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-rerun docs/scootbar/bench/m4-usage-main-3
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.3 [29.6–43.3] | 36.0 [31.6–46.4] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [8.0–17.5] | 16.6 [6.9–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | better |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.6 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-3` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-3 docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 33.4 [28.5–37.7] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.1 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 17.1 [8.2–18.5] | same |
| sway | Idle RSS | 3.5 | 3.3 | better |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.7 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-3` against `m4-usage-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-3 docs/scootbar/bench/m4-usage-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.4 [30.2–39.6] | 33.4 [28.5–37.7] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [7.6–18.6] | 17.1 [8.2–18.5] | same |
| sway | Idle RSS | 3.5 | 3.3 | better |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.7 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-order-3` against `m4-usage-main-3`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-3 docs/scootbar/bench/m4-usage-main-3
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.3 [29.6–43.3] | 33.4 [28.5–37.7] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [8.0–17.5] | 17.1 [8.2–18.5] | same |
| sway | Idle RSS | 3.5 | 3.3 | better |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.7 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```


## The stack as merged against `main` before M4 (the regression this ticket is about)

### `m4-usage-stack` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-stack docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 37.7 [22.9–49.5] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 8.9 [8.8–18.4] | same |
| sway | Idle RSS | 3.5 | 3.7 | same |
| sway | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.1 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

5 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-stack` against `m4-usage-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-stack docs/scootbar/bench/m4-usage-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.4 [30.2–39.6] | 37.7 [22.9–49.5] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.0 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [7.6–18.6] | 8.9 [8.8–18.4] | same |
| sway | Idle RSS | 3.5 | 3.7 | same |
| sway | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.1 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

6 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-stack` against `m4-usage-main-3`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-stack docs/scootbar/bench/m4-usage-main-3
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.3 [29.6–43.3] | 37.7 [22.9–49.5] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.0 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [8.0–17.5] | 8.9 [8.8–18.4] | same |
| sway | Idle RSS | 3.5 | 3.7 | same |
| sway | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.1 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

6 regression(s) beyond the margin.
exit status: 1
```


## The order-file build against the stack as merged

### `m4-usage-order` against `m4-usage-stack`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order docs/scootbar/bench/m4-usage-stack
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 37.7 [22.9–49.5] | 38.5 [30.8–45.9] | same |
| scoot | Idle RSS | 3.7 | 3.3 | better |
| scoot | Idle PSS | 2.2 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 8.9 [8.8–18.4] | 16.7 [3.6–26.4] | same |
| sway | Idle RSS | 3.7 | 3.3 | better |
| sway | Idle PSS | 2.3 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.8 | same |
| sway | CPU while switching workspaces | 0.1 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

### `m4-usage-order-rerun` against `m4-usage-stack`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-rerun docs/scootbar/bench/m4-usage-stack
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 37.7 [22.9–49.5] | 36.0 [31.6–46.4] | same |
| scoot | Idle RSS | 3.7 | 3.3 | better |
| scoot | Idle PSS | 2.2 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 8.9 [8.8–18.4] | 16.6 [6.9–26.4] | same |
| sway | Idle RSS | 3.7 | 3.3 | better |
| sway | Idle PSS | 2.3 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.6 | better |
| sway | CPU while switching workspaces | 0.1 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

### `m4-usage-order-3` against `m4-usage-stack`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-3 docs/scootbar/bench/m4-usage-stack
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 37.7 [22.9–49.5] | 33.4 [28.5–37.7] | same |
| scoot | Idle RSS | 3.7 | 3.3 | better |
| scoot | Idle PSS | 2.2 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 8.9 [8.8–18.4] | 17.1 [8.2–18.5] | same |
| sway | Idle RSS | 3.7 | 3.3 | better |
| sway | Idle PSS | 2.3 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.7 | 3.3 | better |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.7 | same |
| sway | CPU while switching workspaces | 0.1 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```


## The order-file build against M3 post-fix

### `m4-usage-order` against `m3-asahi-clock-bindfix`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order docs/scootbar/bench/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 38.5 [30.8–45.9] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.1 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 16.7 [3.6–26.4] | same |
| sway | Idle RSS | 3.5 | 3.3 | same |
| sway | Idle PSS | 2.1 | 1.9 | better |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```


## Drift: each build against its own first run

### `m4-usage-main-rerun` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-main-rerun docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 31.4 [30.2–39.6] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.0 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 17.6 [7.6–18.6] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.1 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.7 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

### `m4-usage-main-3` against `m4-usage-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-main-3 docs/scootbar/bench/m4-usage-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 40.8 [23.5–41.7] | 30.3 [29.6–43.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.0 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.3 | 0.3 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.4 [8.7–24.6] | 16.9 [8.0–17.5] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.1 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.7 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

### `m4-usage-order-rerun` against `m4-usage-order`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-rerun docs/scootbar/bench/m4-usage-order
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 38.5 [30.8–45.9] | 36.0 [31.6–46.4] | same |
| scoot | Idle RSS | 3.3 | 3.3 | same |
| scoot | Idle PSS | 1.8 | 1.8 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.3 | 3.3 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.7 [3.6–26.4] | 16.6 [6.9–26.4] | same |
| sway | Idle RSS | 3.3 | 3.3 | same |
| sway | Idle PSS | 1.9 | 1.9 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.3 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.6 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

### `m4-usage-order-3` against `m4-usage-order`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-order-3 docs/scootbar/bench/m4-usage-order
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 38.5 [30.8–45.9] | 33.4 [28.5–37.7] | same |
| scoot | Idle RSS | 3.3 | 3.3 | same |
| scoot | Idle PSS | 1.8 | 1.8 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.3 | 3.3 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.7 [3.6–26.4] | 17.1 [8.2–18.5] | same |
| sway | Idle RSS | 3.3 | 3.3 | same |
| sway | Idle PSS | 1.9 | 1.9 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.3 | 3.3 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.7 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```


## Workspaces scope (scoot only, one run each): the order-file build and the stack against `main`

### `m4-usage-ws-order` against `m4-usage-ws-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-ws-order docs/scootbar/bench/m4-usage-ws-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 34.2 [33.0–42.9] | 34.6 [26.3–44.6] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.8 | 1.0 | **REGRESSED** |
| scoot | CPU while switching workspaces | 27.3 | 27.6 | same |
| scoot | Wakeups while switching workspaces | 482 | 482 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-ws-order` against `m4-usage-ws-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-ws-order docs/scootbar/bench/m4-usage-ws-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 41.6 [33.5–49.9] | 34.6 [26.3–44.6] | same |
| scoot | Idle RSS | 3.5 | 3.3 | better |
| scoot | Idle PSS | 2.0 | 1.8 | better |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.3 | better |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 27.8 | 27.6 | same |
| scoot | Wakeups while switching workspaces | 482 | 482 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-ws-stack` against `m4-usage-ws-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-ws-stack docs/scootbar/bench/m4-usage-ws-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 34.2 [33.0–42.9] | 38.1 [33.2–50.2] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.0 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.8 | 1.0 | **REGRESSED** |
| scoot | CPU while switching workspaces | 27.3 | 29.0 | **REGRESSED** |
| scoot | Wakeups while switching workspaces | 482 | 482 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

6 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-ws-stack` against `m4-usage-ws-main-rerun`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-ws-stack docs/scootbar/bench/m4-usage-ws-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 41.6 [33.5–49.9] | 38.1 [33.2–50.2] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.0 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 27.8 | 29.0 | same |
| scoot | Wakeups while switching workspaces | 482 | 482 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

4 regression(s) beyond the margin.
exit status: 1
```

### `m4-usage-ws-main-rerun` against `m4-usage-ws-main`

```text
$ python3 scripts/scootbar-bench/bench.py compare docs/scootbar/bench/m4-usage-ws-main-rerun docs/scootbar/bench/m4-usage-ws-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 34.2 [33.0–42.9] | 41.6 [33.5–49.9] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.0 | 2.0 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.8 | 0.9 | same |
| scoot | CPU while switching workspaces | 27.3 | 27.8 | same |
| scoot | Wakeups while switching workspaces | 482 | 482 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Every row, the runs of a build pooled

The same `pool.py` (verbatim from [`m4-asahi-compares.md`](m4-asahi-compares.md#every-row-the-runs-of-a-build-pooled)), run from the repository root: **the three `main` runs pooled as the baseline, the order-file build's three runs pooled against them**, and the stack as merged (one run) beside it. The pooled verdict is `verdict()` of `scripts/scootbg-bench/report.py` (the margin is the largest of 5% of the baseline's median, the two sides' spread and the unit's floor); the acceptance bar of the ticket asks for it over at least three runs on the CPU row, which this is. What the pairings above say: of the nine order-file-against-`main` compares, seven exit 1 on the **size row alone** and two add **sway's idle CPU** (0.7 against 0.8 ms, against `main`'s second and third runs, whose sway idle CPU is 0.67 and 0.65 where its first run's is 0.86): a row on which `main`'s own three runs span 0.2 ms, which the pooled verdict below calls the same (margin 0.355). The workspaces scope has one run of each build against `main`'s two, pooled at the end.

### scoot: the order-file build (three runs) and the stack as merged, against `main` before M4 (three runs pooled)

```text
$ COMPOSITOR=scoot python3 pool.py . docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3 order=docs/scootbar/bench/m4-usage-order,docs/scootbar/bench/m4-usage-order-rerun,docs/scootbar/bench/m4-usage-order-3 stack=docs/scootbar/bench/m4-usage-stack
### order  (3 run(s) pooled) against docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3
| Startup to first frame | 32.4 [23.5–43.3] | 36.4 [28.5–46.4] | same | margin 37.7 |
| Idle RSS | 3.5 [3.5–3.5] | 3.3 [3.3–3.3] | better | margin 0.176 |
| Idle PSS | 2.0 [2.0–2.1] | 1.8 [1.8–1.8] | better | margin 0.102 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 [0.4–0.4] | same | margin 0.0312 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.3 [3.3–3.3] | better | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 1.0 [0.9–1.0] | same | margin 0.118 |
| CPU while switching workspaces | 0.3 [0.2–0.3] | 0.2 [0.2–0.2] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
1 gated regression(s)

### stack  (1 run(s) pooled) against docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3
| Startup to first frame | 32.4 [23.5–43.3] | 37.7 [22.9–49.5] | same | margin 46.5 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.176 |
| Idle PSS | 2.0 [2.0–2.1] | 2.2 | REGRESSED | margin 0.102 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 0.9 | same | margin 0.1 |
| CPU while switching workspaces | 0.3 [0.2–0.3] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
4 gated regression(s)
```

### sway: the order-file build (three runs) and the stack as merged, against `main` before M4 (three runs pooled)

```text
$ COMPOSITOR=sway python3 pool.py . docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3 order=docs/scootbar/bench/m4-usage-order,docs/scootbar/bench/m4-usage-order-rerun,docs/scootbar/bench/m4-usage-order-3 stack=docs/scootbar/bench/m4-usage-stack
### order  (3 run(s) pooled) against docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3
| Startup to first frame | 17.2 [7.6–24.6] | 16.7 [3.6–26.4] | same | margin 39.8 |
| Idle RSS | 3.5 [3.5–3.5] | 3.3 [3.3–3.3] | same | margin 0.176 |
| Idle PSS | 2.1 [2.1–2.1] | 1.9 [1.9–1.9] | better | margin 0.104 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 [0.4–0.4] | same | margin 0.0312 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.3 [3.3–3.3] | same | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 0.7 [0.7–0.9] | 0.7 [0.6–0.8] | same | margin 0.355 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 [0.2–0.2] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
1 gated regression(s)

### stack  (1 run(s) pooled) against docs/scootbar/bench/m4-usage-main,docs/scootbar/bench/m4-usage-main-rerun,docs/scootbar/bench/m4-usage-main-3
| Startup to first frame | 17.2 [7.6–24.6] | 8.9 [8.8–18.4] | same | margin 26.6 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 | same | margin 0.176 |
| Idle PSS | 2.1 [2.1–2.1] | 2.3 | REGRESSED | margin 0.104 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 | same | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.7 [0.7–0.9] | 0.8 | same | margin 0.204 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.1 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)
```

### scoot, workspaces scope: the order-file build and the stack (one run each) against `main` (two runs pooled)

```text
$ COMPOSITOR=scoot python3 pool.py . docs/scootbar/bench/m4-usage-ws-main,docs/scootbar/bench/m4-usage-ws-main-rerun order=docs/scootbar/bench/m4-usage-ws-order stack=docs/scootbar/bench/m4-usage-ws-stack
### order  (1 run(s) pooled) against docs/scootbar/bench/m4-usage-ws-main,docs/scootbar/bench/m4-usage-ws-main-rerun
| Startup to first frame | 35.4 [33.0–49.9] | 34.6 [26.3–44.6] | same | margin 35.1 |
| Idle RSS | 3.5 [3.5–3.5] | 3.3 | better | margin 0.176 |
| Idle PSS | 2.0 [2.0–2.0] | 1.8 | better | margin 0.101 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.3 | better | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 [0.8–0.9] | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 27.6 [27.3–27.8] | 27.6 | same | margin 1.38 |
| Wakeups while switching workspaces | 482 | 482 | same (not gated) | margin 24.1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
1 gated regression(s)

### stack  (1 run(s) pooled) against docs/scootbar/bench/m4-usage-ws-main,docs/scootbar/bench/m4-usage-ws-main-rerun
| Startup to first frame | 35.4 [33.0–49.9] | 38.1 [33.2–50.2] | same | margin 34 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.176 |
| Idle PSS | 2.0 [2.0–2.0] | 2.2 | REGRESSED | margin 0.101 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 [0.8–0.9] | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 27.6 [27.3–27.8] | 29.0 | REGRESSED | margin 1.38 |
| Wakeups while switching workspaces | 482 | 482 | same (not gated) | margin 24.1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
5 gated regression(s)
```
