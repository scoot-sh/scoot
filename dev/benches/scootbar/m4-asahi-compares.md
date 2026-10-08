# M4 on the Asahi M2: every compare, as run (2026-10-01)

Run from the repository root with the harness of `main` (`7a1f9030a`). `compare NOW BASELINE`; exit status 1 on a gated regression. Rounds one and two (the `m4-asahi-clock-*` runs without `both`) ran the scoot compositor only; round three (`both`) ran scoot and sway, as the M3 post-fix baseline did, for `main`, #367 and the experiment. The pooled verdicts are at the end.

## Against M3 post-fix (rule 1)

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-main dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 30.5 [29.4–38.9] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | better |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-main-rerun dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 37.8 [29.1–48.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 0.9 | better |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-main dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 29.8 [24.1–38.2] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 16.9 [16.6–24.7] | same |
| sway | Idle RSS | 3.5 | 3.5 | same |
| sway | Idle PSS | 2.1 | 2.1 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.7 | better |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr364 dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 31.2 [30.1–47.7] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,514,152 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,381,056 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr366 dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 37.4 [28.8–39.8] | same |
| scoot | Idle RSS | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle PSS | 2.1 | 2.3 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | **REGRESSED** |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,579,720 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,446,624 | **REGRESSED** (not gated) |

5 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367 dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 39.2 [22.7–41.1] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-rerun dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 32.3 [29.7–40.9] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367 dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 38.1 [29.5–48.1] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 17.6 [9.3–18.0] | same |
| sway | Idle RSS | 3.5 | 3.7 | same |
| sway | Idle PSS | 2.1 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.8 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-opt-s dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 29.5 [18.9–33.6] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same |
| any | Bare executable, stripped | 1,249,984 | 1,315,552 | **REGRESSED** (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-opt-s-rerun dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 31.6 [29.2–47.8] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | **REGRESSED** |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.3 | **REGRESSED** |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same |
| any | Bare executable, stripped | 1,249,984 | 1,315,552 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s dev/benches/scootbar/m3-asahi-clock-bindfix
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.0 [24.6–36.8] | 30.4 [25.8–49.4] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.3 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.8 [7.8–25.4] | 17.9 [8.5–18.9] | same |
| sway | Idle RSS | 3.5 | 3.6 | same |
| sway | Idle PSS | 2.1 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.9 | 0.9 | same |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same |
| any | Bare executable, stripped | 1,249,984 | 1,315,552 | **REGRESSED** (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Layer against layer, and the stack total

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr364 dev/benches/scootbar/m4-asahi-clock-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.5 [29.4–38.9] | 31.2 [30.1–47.7] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,514,152 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,381,056 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr366 dev/benches/scootbar/m4-asahi-clock-pr364
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 31.2 [30.1–47.7] | 37.4 [28.8–39.8] | same |
| scoot | Idle RSS | 3.7 | 3.7 | same |
| scoot | Idle PSS | 2.2 | 2.3 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,514,152 | 1,579,720 | same |
| any | Bare executable, stripped | 1,381,056 | 1,446,624 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367 dev/benches/scootbar/m4-asahi-clock-pr366
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 37.4 [28.8–39.8] | 39.2 [22.7–41.1] | same |
| scoot | Idle RSS | 3.7 | 3.7 | same |
| scoot | Idle PSS | 2.3 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,579,720 | 1,645,256 | same |
| any | Bare executable, stripped | 1,446,624 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367 dev/benches/scootbar/m4-asahi-clock-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.5 [29.4–38.9] | 39.2 [22.7–41.1] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | **REGRESSED** |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

3 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-rerun dev/benches/scootbar/m4-asahi-clock-main-rerun
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 37.8 [29.1–48.3] | 32.3 [29.7–40.9] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

2 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367 dev/benches/scootbar/m4-asahi-clock-both-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.8 [24.1–38.2] | 38.1 [29.5–48.1] | same |
| scoot | Idle RSS | 3.5 | 3.7 | same |
| scoot | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | **REGRESSED** |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 16.9 [16.6–24.7] | 17.6 [9.3–18.0] | same |
| sway | Idle RSS | 3.5 | 3.7 | same |
| sway | Idle PSS | 2.1 | 2.2 | **REGRESSED** |
| sway | Idle heap (`RssAnon`) | 0.5 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.5 | 3.7 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.7 | 0.8 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | **REGRESSED** |
| any | Bare executable, stripped | 1,249,984 | 1,512,160 | **REGRESSED** (not gated) |

5 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-opt-s dev/benches/scootbar/m4-asahi-clock-pr367
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 39.2 [22.7–41.1] | 29.5 [18.9–33.6] | same |
| scoot | Idle RSS | 3.7 | 3.6 | same |
| scoot | Idle PSS | 2.2 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,448,648 | better |
| any | Bare executable, stripped | 1,512,160 | 1,315,552 | better (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-opt-s dev/benches/scootbar/m4-asahi-clock-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.5 [29.4–38.9] | 29.5 [18.9–33.6] | same |
| scoot | Idle RSS | 3.5 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.1 | **REGRESSED** |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same |
| any | Bare executable, stripped | 1,249,984 | 1,315,552 | **REGRESSED** (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s dev/benches/scootbar/m4-asahi-clock-both-pr367
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 38.1 [29.5–48.1] | 30.4 [25.8–49.4] | same |
| scoot | Idle RSS | 3.7 | 3.6 | same |
| scoot | Idle PSS | 2.2 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.1 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.3 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| sway | Startup to first frame | 17.6 [9.3–18.0] | 17.9 [8.5–18.9] | same |
| sway | Idle RSS | 3.7 | 3.6 | same |
| sway | Idle PSS | 2.2 | 2.2 | same |
| sway | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| sway | Peak memory (`VmHWM`) | 3.7 | 3.6 | same |
| sway | Idle wakeups per minute | 2 | 2 | same |
| sway | Idle CPU in the window | 0.8 | 0.9 | **REGRESSED** |
| sway | CPU while switching workspaces | 0.2 | 0.2 | same |
| sway | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| sway | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,448,648 | better |
| any | Bare executable, stripped | 1,512,160 | 1,315,552 | better (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

## Noise: a build against its own other runs

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-main-rerun dev/benches/scootbar/m4-asahi-clock-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.5 [29.4–38.9] | 37.8 [29.1–48.3] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 0.9 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-main dev/benches/scootbar/m4-asahi-clock-main
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 30.5 [29.4–38.9] | 29.8 [24.1–38.2] | same |
| scoot | Idle RSS | 3.5 | 3.5 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.5 | 3.5 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 0.9 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same |
| any | Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-rerun dev/benches/scootbar/m4-asahi-clock-pr367
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 39.2 [22.7–41.1] | 32.3 [29.7–40.9] | same |
| scoot | Idle RSS | 3.7 | 3.7 | same |
| scoot | Idle PSS | 2.2 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.0 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367 dev/benches/scootbar/m4-asahi-clock-pr367
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 39.2 [22.7–41.1] | 38.1 [29.5–48.1] | same |
| scoot | Idle RSS | 3.7 | 3.7 | same |
| scoot | Idle PSS | 2.2 | 2.2 | same |
| scoot | Idle heap (`RssAnon`) | 0.5 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.7 | 3.7 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.0 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,645,256 | 1,645,256 | same |
| any | Bare executable, stripped | 1,512,160 | 1,512,160 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-pr367-opt-s-rerun dev/benches/scootbar/m4-asahi-clock-pr367-opt-s
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.5 [18.9–33.6] | 31.6 [29.2–47.8] | same |
| scoot | Idle RSS | 3.6 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.5 | same |
| scoot | Peak memory (`VmHWM`) | 3.6 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.1 | 1.3 | **REGRESSED** |
| scoot | CPU while switching workspaces | 0.2 | 0.2 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,448,648 | 1,448,648 | same |
| any | Bare executable, stripped | 1,315,552 | 1,315,552 | same (not gated) |

1 regression(s) beyond the margin.
exit status: 1
```

```text
$ python3 scripts/scootbar-bench/bench.py compare dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s dev/benches/scootbar/m4-asahi-clock-pr367-opt-s
| Compositor | Row | baseline | now | verdict |
|---|---|---|---|---|
| scoot | Startup to first frame | 29.5 [18.9–33.6] | 30.4 [25.8–49.4] | same |
| scoot | Idle RSS | 3.6 | 3.6 | same |
| scoot | Idle PSS | 2.1 | 2.1 | same |
| scoot | Idle heap (`RssAnon`) | 0.4 | 0.4 | same |
| scoot | Peak memory (`VmHWM`) | 3.6 | 3.6 | same |
| scoot | Idle wakeups per minute | 2 | 2 | same |
| scoot | Idle CPU in the window | 1.1 | 1.1 | same |
| scoot | CPU while switching workspaces | 0.2 | 0.3 | same |
| scoot | Wakeups while switching workspaces | 2 | 2 | same (not gated) |
| scoot | Threads | 1 | 1 | same (not gated) |
| any | Size: stripped binary + non-glibc `ldd` closure | 1,448,648 | 1,448,648 | same |
| any | Bare executable, stripped | 1,315,552 | 1,315,552 | same (not gated) |

0 regression(s) beyond the margin.
exit status: 0
```

## Every row, the runs of a build pooled

Each build's runs as one series, judged against the baseline's runs by
`verdict()` of `scripts/scootbg-bench/report.py` (the margin is the largest of 5%
of the baseline's median, the two sides' spread and the unit's floor). The
script that read the run directories and called `verdict()` is not shipped as
tooling; it is here, as it ran (`[COMPOSITOR=sway] python3 pool.py REPO
BASELINE_DIRS NAME=DIRS ...`, `REPO` the repository root; a baseline or a build
may be several comma-separated run directories).

Idle CPU per run, scoot (ms): `main` 0.926, 0.943, 0.951; #364 0.962; #366
0.979; #367 1.044, 0.993, 1.096; experiment 1.086, 1.302, 1.062; M3 post-fix
1.044. Sway (round three only, one run each): `main` 0.678, #367 0.830,
experiment 0.936; M3 post-fix 0.888.

```python
#!/usr/bin/env python3
"""COMPOSITOR=scoot|sway pool.py REPO BASELINE_DIR NAME=DIR[,DIR...] ... : per-row verdicts of each named build (its runs pooled)
against the baseline's runs, by the repo's own verdict() (scripts/scootbg-bench/report.py)."""
import sys, os
repo = sys.argv[1]
sys.path.insert(0, os.path.join(repo, "scripts", "scootbar-bench"))
sys.path.insert(0, os.path.join(repo, "scripts", "scootbg-bench"))
import tables
import report as scootbg_report

def pooled(dirs):
    runs, metas = [], []
    for d in dirs:
        m, r = tables.load(d); metas.append(m); runs += r
    return metas, runs

COMP = os.environ.get("COMPOSITOR", "scoot")

def rows(metas, runs, comp=None):
    comp = comp or COMP
    out = {}
    for label, row, metric, unit, gated in tables.ROWS:
        s = tables.series(runs, comp, tables.REFERENCE, row, metric, tables._scale(unit))
        if s: out[label] = (s, unit, gated)
    for label, key, unit, gated in tables.STATIC:
        vals = [m["bars"][tables.REFERENCE]["static"].get(key) for m in metas]
        vals = [v for v in vals if v is not None]
        if vals: out[label] = (vals, unit, gated)
    return out

bm, br = pooled(sys.argv[2].split(","))
base = rows(bm, br)
for spec in sys.argv[3:]:
    name, dirs = spec.split("=")
    metas, runs = pooled(dirs.split(","))
    now = rows(metas, runs)
    print(f"\n### {name}  ({len(dirs.split(','))} run(s) pooled) against {sys.argv[2]}")
    reg = 0
    for label, (b, unit, gated) in base.items():
        if label not in now: continue
        n = now[label][0]
        v, margin = scootbg_report.verdict(b, n, unit)
        word = {"beaten": "better", "win": "REGRESSED", "tie": "same"}[v]
        if v == "win" and gated: reg += 1
        print(f"| {label} | {scootbg_report.cell(b, unit)} | {scootbg_report.cell(n, unit)} | {word}{'' if gated else ' (not gated)'} | margin {margin:.3g} |")
    print(f"{reg} gated regression(s)")
```

### scoot: against `main`, its three runs pooled

```text
$ COMPOSITOR=scoot python3 pool-tmp.py . dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main pr364=dev/benches/scootbar/m4-asahi-clock-pr364 pr366=dev/benches/scootbar/m4-asahi-clock-pr366 pr367=dev/benches/scootbar/m4-asahi-clock-pr367,dev/benches/scootbar/m4-asahi-clock-pr367-rerun,dev/benches/scootbar/m4-asahi-clock-both-pr367 opt-s=dev/benches/scootbar/m4-asahi-clock-pr367-opt-s,dev/benches/scootbar/m4-asahi-clock-pr367-opt-s-rerun,dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s

### pr364  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 30.6 [24.1–48.3] | 31.2 [30.1–47.7] | same | margin 41.7 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 | same | margin 0.177 |
| Idle PSS | 2.1 [2.1–2.1] | 2.2 | REGRESSED | margin 0.103 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 | same | margin 0.177 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,514,152 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,381,056 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)

### pr366  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 30.6 [24.1–48.3] | 37.4 [28.8–39.8] | same | margin 35.2 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.177 |
| Idle PSS | 2.1 [2.1–2.1] | 2.3 | REGRESSED | margin 0.103 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 | REGRESSED | margin 0.177 |
| Idle wakeups per minute | 2 [2–2] | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,579,720 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,446,624 | REGRESSED (not gated) | margin 6.25e+04 |
4 gated regression(s)

### pr367  (3 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 30.6 [24.1–48.3] | 38.1 [22.7–48.1] | same | margin 49.6 |
| Idle RSS | 3.5 [3.5–3.5] | 3.7 [3.7–3.7] | same | margin 0.177 |
| Idle PSS | 2.1 [2.1–2.1] | 2.2 [2.2–2.2] | REGRESSED | margin 0.103 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.7 [3.7–3.7] | same | margin 0.177 |
| Idle wakeups per minute | 2 [2–2] | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 1.0 [1.0–1.1] | same | margin 0.128 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 [0.2–0.2] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)

### opt-s  (3 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 30.6 [24.1–48.3] | 29.7 [18.9–49.4] | same | margin 54.7 |
| Idle RSS | 3.5 [3.5–3.5] | 3.6 | same | margin 0.177 |
| Idle PSS | 2.1 [2.1–2.1] | 2.1 [2.1–2.1] | same | margin 0.103 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 [0.4–0.5] | same | margin 0.0219 |
| Peak memory (`VmHWM`) | 3.5 [3.5–3.5] | 3.6 | same | margin 0.177 |
| Idle wakeups per minute | 2 [2–2] | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 0.9 [0.9–1.0] | 1.1 [1.1–1.3] | same | margin 0.265 |
| CPU while switching workspaces | 0.2 [0.2–0.2] | 0.2 [0.2–0.3] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,315,552 | REGRESSED (not gated) | margin 6.25e+04 |
0 gated regression(s)
```

### scoot: against M3 post-fix

```text
$ COMPOSITOR=scoot python3 pool-tmp.py . dev/benches/scootbar/m3-asahi-clock-bindfix main=dev/benches/scootbar/m4-asahi-clock-main,dev/benches/scootbar/m4-asahi-clock-main-rerun,dev/benches/scootbar/m4-asahi-clock-both-main pr364=dev/benches/scootbar/m4-asahi-clock-pr364 pr366=dev/benches/scootbar/m4-asahi-clock-pr366 pr367=dev/benches/scootbar/m4-asahi-clock-pr367,dev/benches/scootbar/m4-asahi-clock-pr367-rerun,dev/benches/scootbar/m4-asahi-clock-both-pr367 opt-s=dev/benches/scootbar/m4-asahi-clock-pr367-opt-s,dev/benches/scootbar/m4-asahi-clock-pr367-opt-s-rerun,dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s

### main  (3 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 31.0 [24.6–36.8] | 30.6 [24.1–48.3] | same | margin 36.4 |
| Idle RSS | 3.5 | 3.5 [3.5–3.5] | same | margin 0.176 |
| Idle PSS | 2.1 | 2.1 [2.1–2.1] | same | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.5 [3.5–3.5] | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 1.0 | 0.9 [0.9–1.0] | better | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 [0.2–0.2] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) | margin 6.25e+04 |
0 gated regression(s)

### pr364  (1 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 31.0 [24.6–36.8] | 31.2 [30.1–47.7] | same | margin 29.7 |
| Idle RSS | 3.5 | 3.7 | same | margin 0.176 |
| Idle PSS | 2.1 | 2.2 | same | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | REGRESSED | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.7 | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 1.0 | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,514,152 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,381,056 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)

### pr366  (1 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 31.0 [24.6–36.8] | 37.4 [28.8–39.8] | same | margin 23.2 |
| Idle RSS | 3.5 | 3.7 | REGRESSED | margin 0.176 |
| Idle PSS | 2.1 | 2.3 | REGRESSED | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | REGRESSED | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.7 | REGRESSED | margin 0.176 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 1.0 | 1.0 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,579,720 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,446,624 | REGRESSED (not gated) | margin 6.25e+04 |
5 gated regression(s)

### pr367  (3 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 31.0 [24.6–36.8] | 38.1 [22.7–48.1] | same | margin 37.5 |
| Idle RSS | 3.5 | 3.7 [3.7–3.7] | same | margin 0.176 |
| Idle PSS | 2.1 | 2.2 [2.2–2.2] | same | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | REGRESSED | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.7 [3.7–3.7] | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 1.0 | 1.0 [1.0–1.1] | same | margin 0.103 |
| CPU while switching workspaces | 0.2 | 0.2 [0.2–0.2] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
2 gated regression(s)

### opt-s  (3 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 31.0 [24.6–36.8] | 29.7 [18.9–49.4] | same | margin 42.7 |
| Idle RSS | 3.5 | 3.6 | same | margin 0.176 |
| Idle PSS | 2.1 | 2.1 [2.1–2.1] | same | margin 0.106 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 [0.4–0.5] | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.6 | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 [2–2] | same | margin 1 |
| Idle CPU in the window | 1.0 | 1.1 [1.1–1.3] | same | margin 0.24 |
| CPU while switching workspaces | 0.2 | 0.2 [0.2–0.3] | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,315,552 | REGRESSED (not gated) | margin 6.25e+04 |
0 gated regression(s)
```

### sway: against `main` (one run each)

```text
$ COMPOSITOR=sway python3 pool-tmp.py . dev/benches/scootbar/m4-asahi-clock-both-main pr367=dev/benches/scootbar/m4-asahi-clock-both-pr367 opt-s=dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s

### pr367  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 16.9 [16.6–24.7] | 17.6 [9.3–18.0] | same | margin 16.8 |
| Idle RSS | 3.5 | 3.7 | same | margin 0.177 |
| Idle PSS | 2.1 | 2.2 | REGRESSED | margin 0.105 |
| Idle heap (`RssAnon`) | 0.5 | 0.4 | same | margin 0.0227 |
| Peak memory (`VmHWM`) | 3.5 | 3.7 | same | margin 0.177 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 0.7 | 0.8 | REGRESSED | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
3 gated regression(s)

### opt-s  (1 run(s) pooled) against dev/benches/scootbar/m4-asahi-clock-both-main
| Startup to first frame | 16.9 [16.6–24.7] | 17.9 [8.5–18.9] | same | margin 18.5 |
| Idle RSS | 3.5 | 3.6 | same | margin 0.177 |
| Idle PSS | 2.1 | 2.2 | same | margin 0.105 |
| Idle heap (`RssAnon`) | 0.5 | 0.4 | same | margin 0.0227 |
| Peak memory (`VmHWM`) | 3.5 | 3.6 | same | margin 0.177 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 0.7 | 0.9 | REGRESSED | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,315,552 | REGRESSED (not gated) | margin 6.25e+04 |
1 gated regression(s)
```

### sway: against M3 post-fix (one run each)

```text
$ COMPOSITOR=sway python3 pool-tmp.py . dev/benches/scootbar/m3-asahi-clock-bindfix main=dev/benches/scootbar/m4-asahi-clock-both-main pr367=dev/benches/scootbar/m4-asahi-clock-both-pr367 opt-s=dev/benches/scootbar/m4-asahi-clock-both-pr367-opt-s

### main  (1 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 16.8 [7.8–25.4] | 16.9 [16.6–24.7] | same | margin 25.7 |
| Idle RSS | 3.5 | 3.5 | same | margin 0.176 |
| Idle PSS | 2.1 | 2.1 | same | margin 0.107 |
| Idle heap (`RssAnon`) | 0.4 | 0.5 | REGRESSED | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.5 | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 | 0.7 | better | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,383,080 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,249,984 | same (not gated) | margin 6.25e+04 |
1 gated regression(s)

### pr367  (1 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 16.8 [7.8–25.4] | 17.6 [9.3–18.0] | same | margin 26.3 |
| Idle RSS | 3.5 | 3.7 | same | margin 0.176 |
| Idle PSS | 2.1 | 2.2 | same | margin 0.107 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.7 | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 | 0.8 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,645,256 | REGRESSED | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,512,160 | REGRESSED (not gated) | margin 6.25e+04 |
1 gated regression(s)

### opt-s  (1 run(s) pooled) against dev/benches/scootbar/m3-asahi-clock-bindfix
| Startup to first frame | 16.8 [7.8–25.4] | 17.9 [8.5–18.9] | same | margin 28 |
| Idle RSS | 3.5 | 3.6 | same | margin 0.176 |
| Idle PSS | 2.1 | 2.2 | same | margin 0.107 |
| Idle heap (`RssAnon`) | 0.4 | 0.4 | same | margin 0.0211 |
| Peak memory (`VmHWM`) | 3.5 | 3.6 | same | margin 0.176 |
| Idle wakeups per minute | 2 | 2 | same | margin 1 |
| Idle CPU in the window | 0.9 | 0.9 | same | margin 0.1 |
| CPU while switching workspaces | 0.2 | 0.2 | same | margin 0.1 |
| Wakeups while switching workspaces | 2 | 2 | same (not gated) | margin 1 |
| Threads | 1 | 1 | same (not gated) | margin 1 |
| Size: stripped binary + non-glibc `ldd` closure | 1,383,080 | 1,448,648 | same | margin 6.92e+04 |
| Bare executable, stripped | 1,249,984 | 1,315,552 | REGRESSED (not gated) | margin 6.25e+04 |
0 gated regression(s)
```
