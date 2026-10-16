---
title: "Golden screenshots per shipped look from the headless harness"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# Golden screenshots per shipped look from the headless harness

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**computer use** (a look is a reproducible screenshot).

## The gap

The only per-look screenshot is a manual script
(`docs/examples/ginger-night/screenshot.sh`). The CI validate gate lives in
[`look-shipped`](../core/look-shipped.md); this ticket adds pixels.

## What to do

A headless harness test per shipped look with a private `XDG_STATE_HOME`
and `XDG_RUNTIME_DIR` (so it can never repoint a real desktop): `scoot
--headless` with fixed clients (a solid-color test client, the bar),
`scoot msg look-apply NAME`, `scoot msg screenshot`, compared with a
committed golden at a small per-channel tolerance; a documented regenerate
command.

## Evidence to record

The goldens, the tolerance and why, and one deliberately broken look shown
failing.

## Not in this ticket

Third-party apps in the screenshots (fonts and versions make them flaky).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
