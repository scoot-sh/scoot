---
title: "HDR readiness probe: report connector HDR capability, refuse HDR with reason"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# HDR readiness probe: report connector HDR capability, refuse HDR with reason

Filed 2026-10-11. Serves **daily-drive**: HDR setup starts with knowing
what the connector can do; not needed for computer use.

Stage 1 of the plan in [hdr-color-management](../resolved/hdr-color-management-done.md)
(which carries the evidence and the fork-separation table).

## The gap

scoot knows nothing about HDR capability: no connector property is ever
read (`HDR_OUTPUT_METADATA`, `Colorspace`, `max bpc`), so on an
HDR-capable output there is neither a report nor a refusal naming why HDR
does not turn on. The pinned Smithay fork (`fdf424d6`) has no
per-connector HDR query (only `vrr_supported`, which also needs
commit-time control a probe does not).

## What to do

Read-only probe per `--tty` connector, reported in the startup log (and
later `scoot msg outputs` if cheap): presence of `HDR_OUTPUT_METADATA`,
the `Colorspace` enum values, the `max bpc` range, and 10-bit mode
availability. Where anything is missing, HDR stays off and the log names
the missing piece — the negative path, verifiable on the dev VM and on
the Asahi panel, lands first. No behavior change on any tier; no fork
change (read-only property reads never enter Smithay's commit path).

## Not in this ticket

Any HDR output (stages 2-4), and `wp_tearing_control_v1` (see `gpu-vrr`).
