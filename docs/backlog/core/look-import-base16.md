---
title: "scoot look import-base16: the one base16-to-token mapping"
status: "open"
area: "core"
priority: "medium"
blocked: "look-format"
---

# scoot look import-base16: the one base16-to-token mapping

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**.

## The gap

The base16 → role mapping exists twice in Nix (`home.nix:571-573`,
`scootbar.nix:482-487`) and nowhere else.

## What to do

`scoot look import-base16 FILE [--name N] [--extends LOOK]` in `scoot-look`,
with the mapping table of the design doc's section 5.3 (today's choices for
the five slots in use; surface = base00, selection = base02,
selection-text = base05, match = base0A, as Stylix's own mako and fuzzel
targets draw; sizes and opacity from the scheme's companion data when
given). Accept base16 YAML and the base24 superset. The result passes the
grammar; a contrast shortfall is a named warning (a look the user chose).
The output is a source look under `user-looks/`.

## Evidence to record

Round-trip tests on a handful of real schemes (licenses checked), and a test
that the mapping equals today's Nix one for the slots it uses.

## Not in this ticket

Nix and Stylix wiring.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
