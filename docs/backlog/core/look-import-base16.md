---
title: "scoot look import-base16: the one base16-to-token mapping"
status: "open"
area: "core"
priority: "medium"
blocked: "look-format"
---

# scoot look import-base16: the one base16-to-token mapping

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**: base16 schemes are the common currency of desktop
palettes, and the Stylix look is built on this importer.

## The gap

The base16 → role mapping exists twice in Nix (`home.nix:571-573`,
`scootbar.nix:482-487`) and nowhere else; a non-Nix user cannot use a
base16 scheme at all.

## What to do

- `scoot look import-base16 FILE [--name N]` in `scoot-look`: the mapping
  of the design doc's §5.3, keeping today's choices so a Stylix user sees
  the same colors (`background = base00`, `foreground = base05`,
  `accent = focus = hover = base0D`, `dim = border = base03`,
  `urgent = base08`), adding `surface = base01`, `selection = base02`, the
  16 terminal hues in base16's terminal order, polarity from `base00`'s
  luminance. Accept base16 YAML and the base24 superset; refuse anything
  else by name.
- The resulting look must pass `validate`; a scheme that fails contrast is
  imported with the failure named, never silently adjusted.

## Evidence to record

Round-trip tests on a handful of real schemes (licenses checked), and a
test that the mapping equals today's Nix one for the five slots it uses.

## Not in this ticket

Nix and Stylix wiring ([`look-stylix-source`](../packaging/look-stylix-source.md)).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
