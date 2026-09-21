---
title: "packages.scoot ships scootctl too, contradicting the documented package split"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# packages.scoot ships scootctl too, contradicting the documented package split

Filed as gh issue #172 (read it — `ls` proof, the honesty argument,
both fix directions; this entry tracks it).

`packages.scoot` has no `cargoBuildFlags`, so `buildRustPackage` builds
the whole workspace and `$out/bin` carries both `scoot` and `scootctl`
(529 KB of closure nobody asked for; `programs.scoot.enable` puts a
redundant second client on `PATH`). The flake's own `scootctl` package
comment ("Just this crate, not the whole workspace") and `docs/nix.md`
("the standalone remote-control client") both disagree with that.

Fix — either direction (decide + record why):
- `cargoBuildFlags = [ "-p" "scoot" ]` in the `scoot` package; or
- keep the workspace build and say so in the `packages` comment and
  `docs/nix.md` ("the compositor, which also ships the `scootctl`
  client").

Verify with `nix build` on both systems (or the reachable ones) +
`ls $out/bin` proof both ways, and check the module wrappers still
resolve (they default `package` to per-system builds).
