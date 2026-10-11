---
title: "Measure nested gles+dma-buf vs pixman on real hardware before auto prefers the GPU under --nested"
status: "research"
area: "core"
priority: "research"
blocked: "renderer-auto-policy"
---

# Measure nested gles+dma-buf vs pixman on real hardware before auto prefers the GPU under --nested

Filed 2026-10-10. Serves **computer use**: `--nested` inside webtop is
how agent sessions run, so the tier `auto` picks there decides every
screenshot and frame of an agent-driven session. Research because the
head-to-head has never been run.

## The gap

`renderer-auto-policy` parks nested `auto` on pixman
(`nested-unmeasured` reason) because the one comparison that exists —
Asahi Test 8, gles read-back vs gles dma-buf — never raced pixman. The
nested GLES path hands each frame to the host as a dma-buf
(`docs/backlog/resolved/nested-dmabuf-present-done.md`), which on real
hardware with a real host compositor could plausibly beat pixman; or the
handoff overhead could lose. `auto` currently assumes the loss without
the number.

## What to do

Run `scripts/nested-dmabuf-bench.sh` on the M2 twice with the same
`HOST_SCOOT`: inner `--renderer pixman` vs inner `--renderer gles` in a
`gpu-scanout` build. Both the host (line 94) and the inner scoot (line
98) are currently hard-coded to `--renderer gles`, so the ticket first
adds an `INNER_RENDERER` variable (default `gles`) and leaves the host as
is. Same >10% rule as the virtual-GPU rematch: flip nested `auto` to
GPU-first only if the gles round's medians beat pixman by >10% over ≥4
rounds, same commit, alternating rounds, medians with spread, raw
`summary.tsv` + logs + SHA recorded. If pixman wins or it is a wash, the
row stays and this entry resolves as decided-and-pinned.

## Not in this ticket

The policy machinery itself (`renderer-auto-policy`); the host-side
renderer choice (stays `gles`); the virtual-GPU rematch
(`renderer-auto-virtual-gpus`); headless (always pixman, measured, not
revisited here).
