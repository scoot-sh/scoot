---
title: "Stylix becomes a synthesized stylix look instead of a layer above the look"
status: "open"
area: "packaging"
priority: "high"
blocked: "look-nix-catalog, look-import-base16"
---

# Stylix becomes a synthesized stylix look instead of a layer above the look

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**. Settles the maintainer's question "do we keep stylix or
create look support with custom looks": both, with Stylix as a source of a
look (design doc §5, Decision D5).

## The gap

Stylix sits above the look (`scootbar.nix:7-20`): its values are baked into
the config files as ordinary settings, which a runtime look layer sits
under, so a swap could never change a key Stylix set. The mapping is
duplicated (`home.nix:571-573`, `scootbar.nix:482-487`).

## What to do

- One Nix function (`nix/lib/look-from-stylix.nix`) that serializes
  `config.lib.stylix.colors.withHashtag.base00..base0F`, the fonts and
  sizes, opacity, cursor and image into the store, and a derivation that
  runs `scoot look import-base16` on it: the `stylix` look.
- With Stylix present and on, the catalog gains `stylix` and the declared
  default becomes `stylix` at `mkDefault` (an explicit `look` wins).
- Delete the per-module Stylix leaf mappings in `home.nix` and
  `scootbar.nix`. Remove `programs.scootbar.stylix.enable`;
  `programs.scoot.stylix.enable` now means "synthesize the `stylix` look";
  `programs.scoot.stylix.wallpaper.enable` means "the `stylix` look carries
  `stylix.image`". Breaking (`feat(nix)!`), the maintainer's config updated
  in step, and the one behavior flip stated in the changelog: with Stylix on
  *and* an explicit `look`, the look now wins.
- The other first-party app modules (scootlaunch, scootnotify) never read
  Stylix themselves; they read the look layer.
- Stylix is never a flake input or a hard dependency.
- Docs in the same PR: rewrite `site/.../scoot/theming.md`'s precedence
  (38-44) and Stylix (158-189) sections and `site/.../scootbar/theming.md`'s.

## Evidence to record

Each Stylix read beyond today's stub (`base01`, `02`, `04`, `06`, `07`,
`09`-`0F`, `fonts.monospace`, `opacity`, `polarity`) checked by hand against
a named Stylix rev and added to the stub in `nix/tests.nix:754-807`, with the
rev recorded in a header like `nix/scootbar-tests.nix:9-15`. A test that the
`stylix` look's colors equal what today's leaves produce.
`scripts/scootbar-stylix-test.sh` (the real-Stylix run) green.

## Not in this ticket

Stylix targets for apps scoot does not theme (starship, Helix, btop): left
to Stylix's own targets.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
