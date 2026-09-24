---
title: "A/B resource usage: scoot vs niri on identical workloads"
status: "open"
area: "testing"
priority: "medium"
blocked: "run after the remaining unblocked GPU-tier tickets land (user request 2026-09-23)"
---

# A/B: scoot vs niri

User request 2026-09-23: "Do we have any sense of resource usage of niri vs
scoot? Can we A/B them once we have enough gpu tickets done?" We have none:
every scoot number so far is scoot-vs-scoot (pixman vs GLES scanout, Asahi
Test 4). niri is the reference point `README.md` compares itself to, so the
claim "lightweight, fast" needs a measured answer against it.

## What to measure (same machine, same workload, alternated runs)

- **Idle:** RSS/PSS after settle, CPU jiffies over a fixed window, wakeups/s.
- **Under damage:** compositor CPU during continuous pointer motion (large
  jumps, not a small cursor move), during a relayout storm (focus/move
  column cycling), and with a client animating at 60 Hz.
- **Latency:** input → frame presented (presentation-time), where both
  expose it.
- **Startup:** time to first frame; binary size and linked libraries.
- **Screenshot cost** (computer use): latency and CPU per capture via each
  compositor's own path and via `grim`.

## Fairness rules

- Same clients (`foot` ×N), same output mode, same scale, same config
  shape as far as both allow (gaps, borders, no animations — niri animates
  by default; turn it off, and record a second run with its defaults).
- Same input source for both: kernel-level injection (`uinput`/`ydotool`)
  on `--tty`, not scoot's IPC injection, so neither gets a private fast path.
- Report every scoot tier separately: pixman (default), `--renderer gles`,
  and the `gpu-scanout` build. niri is GLES-only, so on the dev VM (llvmpipe)
  it and scoot-gles both pay software rasterisation; the pixman row is the
  honest GPU-less comparison.
- Alternate binaries, ≥3 rounds, wait for idle before sampling, record raw
  numbers, commands and exact versions (niri from nixpkgs, scoot by SHA).
- Dev VM first; a real-GPU run is an `Asahi.md` runbook step for the user.

## Licensing

niri is GPL-3.0: running it for measurement is fine; copying its code or
config into scoot is not (`CLAUDE.md`).

## Output

A results section in a new `docs/benchmarks.md` (or `Asahi.md` for the
hardware half), and a short, honest README line only if the numbers
support one — README is for users and prospective users.
