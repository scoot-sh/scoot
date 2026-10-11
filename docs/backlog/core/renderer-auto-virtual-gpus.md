---
title: "Measure pixman vs GPU scanout on virgl/venus and vmwgfx before auto prefers the GPU there"
status: "research"
area: "core"
priority: "research"
blocked: "renderer-auto-policy"
---

# Measure pixman vs GPU scanout on virgl/venus and vmwgfx before auto prefers the GPU there

Filed 2026-10-10. Serves **daily-drive**: VM users on a host-backed
virtual GPU should get whichever tier is actually faster, not whichever
reads better on paper. Research because the measurement needs hardware
no box in this project currently has.

## The gap

`renderer-auto-policy` parks `virtio_gpu` and `vmwgfx` on pixman
(`virtual-gpu` reason) on thin evidence: virtio-gpu *without* virgl is
measured (GPU scanout ~1.5x dearer than dumb buffers,
`docs/backlog/resolved/gpu-vs-cpu-measured-done.md`), but virgl/venus
(virtio_gpu with a host GPU behind it) and vmwgfx SVGA3D are unmeasured —
a real 3D engine behind the virtual device could flip the result, and
`auto` would then be slow by design for exactly the users who have a GPU.

## What to do

Run `scripts/tty-tier-bench.sh` in a QEMU guest with `-device
virtio-gpu-gl` (virgl or venus) and in a VMware guest for vmwgfx, same
commit, one binary, alternating rounds, medians with spread (never
best-of), idle-settled, tier confirmed from the log line before trusting
a number: `SCOOT_DUMB=$BIN SCOOT_GPU=$BIN OUT=/tmp/... ROUNDS=4
BACKEND=--tty scripts/tty-tier-bench.sh` (the `TIER_A_ARGS`/`TIER_B_ARGS`
overrides from `renderer-auto-policy` select the tiers). Record SHA,
commands, raw `summary.tsv` and logs. Flip a row from `Pixman` to `TryGpu`
only if the GPU round's median `move` and `width` j/ev both beat pixman
by >10% over ≥4 rounds. No such host exists in this project today — say
so in the ticket if that is still true, and leave the rows parked.

## Not in this ticket

The policy machinery itself (`renderer-auto-policy`); the nested
rematch (`renderer-auto-nested`); changing the un-virgl'd virtio_gpu row
(measured, stays pixman); any host-side setup beyond what the bench
needs.
