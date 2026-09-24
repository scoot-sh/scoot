---
title: "A/B resource usage: scoot vs niri on identical workloads (dev VM half) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A/B: scoot vs niri — RESOLVED (dev VM half)

RESOLVED 2026-09-24 (PR #237). The ticket was blocked on "after the remaining
unblocked GPU-tier tickets"; the user then asked for it directly ("Do we
have any sense of resource usage of niri vs scoot? Can we A/B them?"). The
results, method and every caveat are in [`docs/benchmarks.md`](../../benchmarks.md);
the harness is `scripts/niri-ab-bench.sh` with its helpers in
`scripts/niri-ab/`. The real-GPU half, `--tty` included, is its own open
item: [`testing/niri-ab-real-gpu.md`](../testing/niri-ab-real-gpu.md)
(`Asahi.md` Test 9).

What the VM could and could not do, against the ticket as written below:

- niri renders only through GLES and refuses llvmpipe on `--tty`, so both
  compositors ran **nested**, one at a time, in the same host (cage,
  headless, pixman, `-d`). Same output size (1600x1000, verified from each
  compositor's own output query), same clients (`foot` ×3, empty config),
  three rotating rounds, idle-settled before every scene.
- **Same input source:** not `uinput` (a headless host has no libinput), but
  one persistent `zwlr_virtual_pointer_v1` device injecting into the *host*
  (`scripts/niri-ab/vptr`). `wlrctl` could not do it: a device per event
  toggles the host seat's pointer capability, and 60 moves delivered zero
  `wl_pointer.motion`.
- **Latency** (input → present) was not measured: nested, no signal is
  common to both, and scoot draws no nested pointer. Moved to the real-GPU
  item.
- **gpu-scanout** was not a separate row: nested in a host without
  `linux-dmabuf` it presents by read-back like the default build
  (`nested/gpu.rs`, `try_negotiate`); the real-GPU item's Part A gives the
  host GLES so that it comes up.

Found on the way, and filed:
[GLES captures leak a frame each on a static screen](./gles-capture-leaks-a-frame-per-shot-done.md)
(high; fixed by PR #238), and
[nested scoot presents fewer frames than niri for a ~60 Hz client](../core/nested-frame-rate-vs-client.md)
(low). A harness artifact was also caught before it reached the results: a
niri config on the VM's 9p mount cost niri ~55 wakeups/s at idle.

---

The ticket as filed:

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
