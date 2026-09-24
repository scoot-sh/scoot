---
title: "A/B resource usage vs niri on a real GPU, --tty included (Asahi.md Test 9)"
status: "open"
area: "testing"
priority: "medium"
blocked: "needs the user's Apple Silicon machine: Asahi.md Test 9"
---

# A/B vs niri on a real GPU

This is the half of the [niri A/B](../resolved/niri-ab-benchmark-done.md)
that the dev VM cannot run. On the VM, niri runs only nested (it refuses
llvmpipe on `--tty`), and every GLES number is software rasterisation. The
VM results and their caveats are in `docs/benchmarks.md`.

The runbook is `Asahi.md` Test 9:

- **Part A** needs no VT. It runs `scripts/niri-ab-bench.sh` with
  `HOST_RENDERER=gles2`, so the nested niri and scoot-gles render on the
  AGX, and the `gpu-scanout` build presents by dma-buf.
- **Part B** needs a VT. Each compositor runs as the session in turn
  (scoot dumb + pixman, scoot `gpu-scanout` `--renderer gles`, niri). From
  a `foot` inside the session, `scripts/niri-ab/sample.sh session …` counts
  CPU, wakeups and memory under idle, IPC relayout, kernel-level pointer
  motion (ydotool) and screenshots.

Still open after the VM half, and to answer here:

- per-frame GLES cost on real hardware, niri against scoot-gles and
  scoot-gpu;
- the pointer on a real session, where both compositors draw one (nested,
  only niri did);
- input-to-present latency. It was not measured on the VM because no signal
  there is common to both compositors. A real `--tty` run can
  compare presentation-time feedback against the ydotool send time. The
  method for that is still to be decided.
