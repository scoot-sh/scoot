---
title: "Measure nested gpu+dma-buf vs cpu on real hardware before auto picks the gpu under --nested"
status: "research"
area: "core"
priority: "research"
blocked: null
---

# Measure nested gpu+dma-buf vs cpu on real hardware before auto picks the gpu under --nested

Filed 2026-10-10. Serves **computer use**: `--nested` inside webtop is
how agent sessions run, so the tier `auto` picks there decides every
screenshot and frame of an agent-driven session. Research because the
head-to-head has never been run.

## The gap

`renderer-auto-policy` parks nested `auto` on cpu
(`nested-unmeasured` reason) because the one comparison that exists —
Asahi Test 8, GLES read-back vs GLES dma-buf — never raced cpu. The
nested gpu path hands each frame to the host as a dma-buf
(`docs/backlog/resolved/nested-dmabuf-present-done.md`), which on real
hardware with a real host compositor could plausibly beat cpu; or the
handoff overhead could lose. `auto` currently assumes the loss without
the number.

## What to do

Run `scripts/nested-dmabuf-bench.sh` on the M2 twice with the same
`HOST_SCOOT`: inner `--renderer cpu` vs inner `--renderer gpu` in a
`gpu-scanout` build. Both the host (line 94) and the inner scoot (line
98) are currently hard-coded to `--renderer gles`, so the ticket first
adds an `INNER_RENDERER` variable (default `gles`) and leaves the host as
is. Same >10% rule as the virtual-GPU rematch: flip nested `auto` to
the gpu tier only if the gpu round's medians beat cpu by >10% over ≥4
rounds, same commit, alternating rounds, medians with spread, raw
`summary.tsv` + logs + SHA recorded. If cpu wins or it is a wash, the
row stays and this entry resolves as decided-and-pinned.

## Not in this ticket

The policy machinery itself (`renderer-auto-policy`); the host-side
renderer choice (stays `gpu`); the virtual-GPU rematch
(`renderer-auto-virtual-gpus`); headless (always cpu, measured, not
revisited here).
