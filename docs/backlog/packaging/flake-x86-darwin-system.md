---
title: "The flake's `systems` list still names `x86_64-darwin`, which the pinned nixpkgs refuses to evaluate at all (LOW, pre-existing)."
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# The flake's `systems` list still names `x86_64-darwin`, which the pinned nixpkgs refuses to evaluate at all (LOW, pre-existing).

The flake's `systems` list still names `x86_64-darwin`, which the pinned
nixpkgs refuses to evaluate at all (LOW, pre-existing). Found while
adding item 16's `packages` output: `nix flake check --all-systems` fails
on `packages.x86_64-darwin.default` — and equally on
`devShells.x86_64-darwin.default`, i.e. it predates the new outputs rather
than being introduced by them. Verified against `main` itself, not just by
reading: `nix eval
'git+file:///Users/steveyackey/code/flexwm?ref=main#devShells.x86_64-darwin.default.name'`
throws out of `nixpkgs.legacyPackages.x86_64-darwin`, nixpkgs having
dropped that platform after the 26.05 branch. Plain `nix flake check`
(current system only) passes, on both macOS and the dev VM. The fix is a
one-line choice — drop `x86_64-darwin` from `systems`, or repin nixpkgs to
a branch that still carries it — and it only matters if an Intel Mac ever
has to build this; left alone to keep item 16 to its own scope.
