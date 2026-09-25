---
title: "A/B resource usage vs niri on a real GPU, --tty included (Asahi.md Test 9)"
status: "open"
area: "testing"
priority: "low"
blocked: "input-to-present latency needs a measurement method first; everything else ran 2026-09-25"
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

## Run 2026-09-25: everything except latency

`Asahi.md` Test 9 ran on the Apple M2 (`main` at `e1dce6f`, niri 26.04).
The results are in `Asahi.md` and in `docs/benchmarks.md`'s real-GPU
section. Against the list above:

- **Per-frame GLES cost: answered.** Nested, niri costs 0.94 ms per
  relayout frame and 2.35 ms per animated frame. scoot-gles on the
  read-back path costs 2.53 and 2.47 ms, and scoot-pixman 8.78 and 7.10 ms.
  The gpu-scanout build presenting by dma-buf is the cheapest in total CPU
  (relayout 250 ms against niri's 580; animate 820 against 1290). That
  variant has no DIAG frame counts.
- **The pointer on a real session: answered.** On `--tty`, where both
  compositors draw a pointer and neither has a cursor plane, scoot-gpu
  takes 9.2–9.6% of a core, niri 22–23.5% and scoot-pixman 32–35%, at
  ydotool's rate.
- **Input-to-present latency: still open.** No method was settled, and it
  was not attempted. This is the only thing the ticket still asks for, so
  it drops to low. The obvious method: timestamp each ydotool send, and
  take the `wp_presentation` `presented` time of the first frame that
  moved a client-visible cursor. That needs a client that commits on
  pointer motion, in both compositors.
