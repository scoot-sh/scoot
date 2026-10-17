---
title: "Retire the Nix look registry once every consumer reads look directories"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-16"
---

# Retire the Nix look registry once every consumer reads look directories

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**: one source of truth.

## The gap

Until every consumer reads look directories, the Nix registry
(`desktop.nix:1931-2147`) and `theme-look.nix` stay, and `look-shipped`'s
equality check keeps them in step.

## What to do

Once the catalog, the Stylix source, the toolkit work, the three leaves
tickets and the greeter have landed: delete the registry, `theme-look.nix`'s
look-reading helpers and the equality check; the per-look pins in
`nix/tests.nix` read `looks/*/look.toml`.

## Evidence to record

`nix flake check` green with no registry; a grep showing no reader left.

## Resolution (2026-10-17, PR #TBD)

Deleted `desktop.nix`'s hand-maintained `looks` table (217 lines),
`nix/modules/theme-look.nix` (its readers move to the new helper) and
`nix/looks-shipped-check.nix` (agreement holds by construction). New
`nix/lib/look-shipped.nix` derives the same record shape from
`looks/<name>/look.toml` (palette, polarity, fonts, night temperature,
cursor) plus the example files by convention (app files where the
example ships them, wallpaper from `docs/assets/wallpapers/<name>.png`
then `docs/examples/<name>/wallpaper.png`, else null), and carries the
one look-reading helper (`lookFor`, `themed`, the hex spellings, the
app colors, Mocu/Adwaita) every slot reads through. The greeter, the
catalog defaults and the Stylix base look read the same map.
`nix/looks-examples-check.nix` keeps the pins that still have two
sides: each manifest against its example files (style tokens, foot
hues, cursor, wallpaper file). No user-facing option goes, so no
BREAKING marker.

Behavioral proof: the check derivations' `buildCommand` diff before
and after shows only the edited `.nix` sources' own store paths --
every generated file's store path (and so its content) is identical --
on `aarch64-linux` and `x86_64-linux`; `scootbar-modules` and the
darwin `scoot-modules` drvPaths are unchanged. A one-hue manifest
mutation fails eval naming the drift; a one-digit palette mutation
moves the check drvPath. Full report in the scratchpad
(`W5-nix-retire-registry-report.md`).

## Not in this ticket

Anything behavioral.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
