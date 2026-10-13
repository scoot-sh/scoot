---
title: "Golden screenshots per shipped look from the headless harness, and the validate gate in CI"
status: "open"
area: "testing"
priority: "medium"
blocked: "look-ipc"
---

# Golden screenshots per shipped look from the headless harness, and the validate gate in CI

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**computer use** (a look is a reproducible screenshot) and keeps the
shipped looks honest.

## The gap

`site/src/content/docs/scoot/theming.md:208-209` says "a check renders every
look into every target and fails on a missing role"; no such check exists.
`nix/tests.nix` has five hand-written evaluations (3940-3968) and an AA check
over `gtk.css` only (14907-14932). The only per-look screenshot is a manual
script (`docs/examples/ginger-night/screenshot.sh`).

## What to do

- CI: `scoot look validate` over every shipped look on any change to them
  (with the CI path filter from [`look-format`](../core/look-format.md)).
- A headless harness test per shipped look: `scoot --headless` with fixed
  clients (a solid-color test client, the bar), `scoot msg look-apply NAME`,
  `scoot msg screenshot`, compared with a committed golden at a small
  per-channel tolerance; a documented regenerate command.
- Make the theming page's claim true, or reword it, in the same PR.

## Evidence to record

The goldens themselves, the comparison's tolerance and why, and one
deliberately broken look shown failing the gate.

## Not in this ticket

Third-party apps in the screenshots (fonts and versions make them flaky).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
