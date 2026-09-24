---
title: "Nested scoot presents fewer frames than niri for a ~60 Hz client"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Nested scoot presents fewer frames than niri for a ~60 Hz client

Found 2026-09-24 by the scoot/niri A/B (`docs/benchmarks.md`, "animate"
scene), dev VM, release build of `fe41921`. A `foot` printing a line about
every 16 ms (`sleep 0.016` plus a fork per line) was presented at these
rates, counted as the nested compositor's commits in the host's protocol
log over 10 s (median of three rounds):

| | frames/s | compositor CPU |
|---|---|---|
| scoot --nested (pixman) | 49.8 [49.7–49.8] | 12.2% of a core |
| scoot --nested --renderer gles | 43.0 [42.8–43.0] | 78.8% |
| niri 26.04, nested (llvmpipe) | 54.1 [54.0–54.2] | 80.1% |

Raw data is on the dev VM, in `~/evidence/niri-ab/run-diag/results.tsv`
(`frames` column) and `run-main/results.tsv` (CPU).

The pixman session is far from CPU-bound, so its ~8% shortfall against niri
looks like pacing: when frame callbacks reach the client relative to the
host's frame, and so how many client commits share a presented frame. It
does not look like rendering cost. The gles figure is probably bounded by
llvmpipe's cost per frame (18.4 ms), and it needs to be separated from the
pacing question before anyone draws conclusions from it.

Not investigated. The first step is to log per-frame timestamps of
client commits, scoot's frame callbacks and the host's `wl_callback.done`
for one run of each (the benchmark's `DIAG=1` host log already has the host
side), and check whether scoot's callbacks lag a frame behind niri's.
Priority is low because the visible effect is about 4 fewer frames a second
for one kind of client in `--nested`. Check `--tty` before acting on this:
its presentation is paced differently (vblank-driven).
