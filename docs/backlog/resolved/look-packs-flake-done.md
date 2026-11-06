---
title: "Later: flake lookPacks output for programs.scoot.looks"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "later"
resolved: "2026-11-06"
---

# Later: flake lookPacks output for programs.scoot.looks

Filed 2026-11-06, split out of [`look-share`](resolved/look-share-done.md) (whose
git half shipped as `scoot look install GIT-URL`). Child of
[`live-looks`](live-looks.md). **Later**: not scheduled. Serves
**daily-drive**: a Nix user shares or consumes looks as a flake input
instead of copying directories.

## The gap

`programs.scoot.looks` takes shipped look names (strings) or *one* source
look directory per entry (`nix/look-catalog.nix`, `toEntry`). A flake
input is an attribute set with an `outPath`, so `inputs.foo + "/looks/a"`
evaluates to a string (checked with `nix eval`: `typeOf` is `"string"`,
where `inputs.foo.outPath + "/looks/a"` is a `path`), and only the
absolute-store-path-string form of an entry works today. There is no way to
name a directory that *holds* looks, so a repository with five looks needs
five hand-written paths, and nothing says which flake outputs are looks.

## What to do

- A flake output convention, `lookPacks.<name>`: a directory whose
  subdirectories are source looks (each with a `look.toml`), buildable
  with no Nix evaluation of its own.
- `programs.scoot.looks` accepts a pack (a path or a flake input's
  `lookPacks.<name>`) and expands it to one entry per look, with
  `toEntry`'s checks per look and a loud refusal for an empty pack or a
  name that two entries share.
- `nix/tests.nix` cases: a pack of two looks, a pack with a duplicate name,
  a pack with a directory that is not a look, a pack next to a shipped name.
- Document it in the Nix section of the looks page.

Nix-declared looks are the user's own declared set (origin `declared`), not
`installed`, so a pack is as trusted as the flake input itself and takes no
`--trust` review. Say so in the docs.

## Not in this ticket

A central registry (D6: none). `scoot look install GIT-URL` (shipped).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution

Landed 2026-11-06. `programs.scoot.looks` takes a look pack: a directory
without a `look.toml` expands (`nix/look-catalog.nix`, `packEntries`) to
one entry per subdirectory, each through `toEntry`. The flake exports
`lookPacks.shipped = ./looks`. Decisions the entry left open: a
subdirectory that is not a look refuses (files and dot-directories are
skipped, symlinks not followed); a name two different sources claim
refuses when either is from a pack, while pack-free lists keep first-wins;
a pack look named `stylix` against the synthesized `stylix` is not checked
(a single path entry has the same gap). Checks: `nix/tests.nix` (pack
fixtures in `nix/test-look-packs/`).
