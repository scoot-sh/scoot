---
title: "scoot look import-base16: the one base16-to-token mapping"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
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

## Resolution

2026-10-15. `scoot-look import-base16 FILE [--name N] [--extends LOOK]
[--out DIR] [--origin O]` in the `scoot-look` crate (`crates/scoot-look/src/import.rs`,
pure library plus the CLI wiring in `src/main.rs`): the section 5.3
mapping, base16 YAML (nested `palette:` or flat) and the base24
superset plus the JSON form the Nix side serializes, companion
fonts/sizes/opacity/cursor when given, a complete look without
`--extends` and a palette-only child with it. The output re-parses;
contrast warns, never refuses, for the default user origin. Evidence:
round-trips over four published schemes (3024, Solarized Dark/Light,
base24 3024 Day; MIT, authors in the test header) and a test pinning
the mapping to today's Nix slots (`home.nix:571-573`,
`scootbar.nix:482-487`); nextest 85/85 and `cargo test` green on macOS
and Linux, clippy and fmt clean both. Site reference in
`site/src/content/docs/reference/look-format.md`. PR:
https://github.com/scoot-sh/scoot/pull/611 .
