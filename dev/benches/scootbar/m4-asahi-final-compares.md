# M4 final stack on the Asahi M2: every compare, as run (2026-10-01)

Run from the repository root with the harness of `main` (`98c4b7a32`, the harness files byte-identical to `7a1f9030a`'s). `compare NOW BASELINE`; exit status 1 on a gated regression. Scoot and sway in every run. The pooled verdicts are at the end; the script is `pool.py` of [`m4-asahi-compares.md`](m4-asahi-compares.md#every-row-the-runs-of-a-build-pooled).

## The final #367 tip against the final `main` (first run)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-pr367 dev/benches/scootbar/m4-asahi-final-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.8 [24.7–44.7] | 29.0 [28.9–37.5] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.7 [8.0–25.1] | 17.2 [7.7–17.6] | same |
| sway | Idle RSS | 3.5 | 3.6 | same |
| sway | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

3 regression(s) beyond the margin.
exit status: 1
```

## The final #367 tip against the final `main` (the A-B-A rerun)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-pr367 dev/benches/scootbar/m4-asahi-final-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.4 [15.7–38.3] | 29.0 [28.9–37.5] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.8 [8.4–18.7] | 17.2 [7.7–17.6] | same |
| sway | Idle RSS | 3.5 | 3.6 | same |
| sway | Idle PSS | 2.2 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.6 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

4 regression(s) beyond the margin.
exit status: 1
```

## The final #367 tip against M3 post-fix

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-pr367 dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 29.0 [28.9–37.5] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | better |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 17.2 [7.7–17.6] | same |
| sway | Idle RSS | 3.5 | 3.6 | same |
| sway | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

3 regression(s) beyond the margin.
exit status: 1
```

## Final `main` (first run) against M3 post-fix

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-main dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 31.8 [24.7–44.7] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 17.7 [8.0–25.1] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.1 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Final `main` (rerun) against M3 post-fix

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-main-rerun dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 29.4 [15.7–38.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 17.8 [8.4–18.7] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.6 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Drift: the final `main` rerun against its first run (A-B-A)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-main-rerun dev/benches/scootbar/m4-asahi-final-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.8 [24.7–44.7] | 29.4 [15.7–38.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.7 [8.0–25.1] | 17.8 [8.4–18.7] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.6 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Final `main` (first run) against the earlier M4 `main` (round three, scoot and sway)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-main dev/benches/scootbar/m4-asahi-clock-both-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.8 [24.1–38.2] | 31.8 [24.7–44.7] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [16.6–24.7] | 17.7 [8.0–25.1] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.1 | same |
| sway | Idle heap (`RssAnon`) | 0.5 | 0.4 | better |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

## Final `main` (rerun) against the earlier M4 `main` (round three)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-main-rerun dev/benches/scootbar/m4-asahi-clock-both-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.8 [24.1–38.2] | 29.4 [15.7–38.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [16.6–24.7] | 17.8 [8.4–18.7] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.5 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.6 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## The final #367 tip against the earlier #367 (round three, `b6dee9710`)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-final-pr367 dev/benches/scootbar/m4-asahi-clock-both-pr367
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 38.1 [29.5–48.1] | 29.0 [28.9–37.5] | same |
| scoot | Idle RSS | 3.7 | 3.6 | same |
| scoot | Idle PSS | 2.2 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.4 | better |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.1 | 0.9 | better |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [9.3–18.0] | 17.2 [7.7–17.6] | same |
| sway | Idle RSS | 3.7 | 3.6 | same |
| sway | Idle PSS | 2.2 | 2.3 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.7 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Every row, the runs of a build pooled

The same `pool.py` (verbatim from [`m4-asahi-compares.md`](m4-asahi-compares.md#every-row-the-runs-of-a-build-pooled)), the baseline the two `main` runs pooled. The #367 tip has **one** run, so its pooled verdict is one sample against two, which is weak: do not read a "same" from a single sample as more than that. The new #367 run is not pooled with the earlier #367 runs (`b6dee9710`: the code differs).

### scoot: the final #367 tip against the two final `main` runs pooled

```text
$ COMPOSITOR=scoot python3 pool-tmp.py . dev/benches/scootbar/m4-asahi-final-main,dev/benches/scootbar/m4-asahi-final-main-rerun pr367=dev/benches/scootbar/m4-asahi-final-pr367

### pr367  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-final-main,dev/benches/scootbar/m4-asahi-final-main-rerun
| Startup to first frame | 30.7 [15.7–44.7] | 29.0 [28.9–37.5] | same | margin 37.6 |
| Idle RSS | 3.5 [3.5–3.5] | 3.6 | same | margin 0.176 |
| Idle PSS | 2.1 [2.1–2.1] | 2.2 | REGRESSED | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 | same | margin 0.0215 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.6 | same | margin 0.176 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 1.0 [1.0–1.0] | 0.9 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)
```

### sway: the final #367 tip against the two final `main` runs pooled

```text
$ COMPOSITOR=sway python3 pool-tmp.py . dev/benches/scootbar/m4-asahi-final-main,dev/benches/scootbar/m4-asahi-final-main-rerun pr367=dev/benches/scootbar/m4-asahi-final-pr367

### pr367  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-final-main,dev/benches/scootbar/m4-asahi-final-main-rerun
| Startup to first frame | 17.7 [8.0–25.1] | 17.2 [7.7–17.6] | same | margin 27 |
| Idle RSS | 3.5 [3.5–3.5] | 3.6 | same | margin 0.175 |
| Idle PSS | 2.2 [2.1–2.2] | 2.3 | REGRESSED | margin 0.108 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 | same | margin 0.0215 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.6 | same | margin 0.175 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.7 [0.6–0.8] | 0.8 | same | margin 0.198 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)
```

### The same, with the earlier M4 `main` runs (rounds one to three, 3 runs) added to the baseline: five `main` runs, scoot

```text
$ COMPOSITOR=scoot python3 pool-tmp.py . [five main dirs] pr367=dev/benches/scootbar/m4-asahi-final-pr367

### pr367  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-final-main,dev/benches/scootbar/m4-asahi-final-main-rerun,dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 30.6 [15.7–48.3] | 29.0 [28.9–37.5] | same | margin 41.3 |
| Idle RSS | 3.5 [3.5–3.5] | 3.6 | same | margin 0.177 |
| Idle PSS | 2.1 [2.1–2.1] | 2.2 | REGRESSED | margin 0.103 |
| Idle heap (`RssAnon`) | 0.4 [0.4–0.4] | 0.4 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.6 | same | margin 0.177 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 1.0 [0.9–1.0] | 0.9 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)
```
