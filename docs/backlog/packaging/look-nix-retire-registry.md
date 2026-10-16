---
title: "Retire the Nix look registry once every consumer reads look directories"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-nix-catalog, look-stylix-source, look-leaves-terminal"
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

## Not in this ticket

Anything behavioral.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
