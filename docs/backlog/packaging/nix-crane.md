---
title: "Nix: crane, so compiled dependencies are cached and CI rebuilds only scoot's crates"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Nix: crane, so compiled dependencies are cached and CI rebuilds only scoot's crates

Filed 2026-10-03, split out of
[nix-publishing](../resolved/nix-publishing-done.md) (which covers the Cachix and FlakeHub
half). Serves **daily-drive**: every merge to `main` recompiles Smithay from
scratch once per packaged binary, so the packaged build stays the slowest
job in CI and `scoot-gpu` stays too expensive to build there.

Blocked on `nix-publishing` in the payoff sense, not the work sense: the
crane move can land any time, but it pays off once Cachix receives pushes,
because the compiled-dependency store path is what Cachix keeps and every
build reuses.

## The gap

The packages are `buildRustPackage` derivations, which compile the whole
dependency graph (the `scoot-sh/smithay` fork and the `scoot-sh/wayland-rs`
`wayland-backend` patch included) inside each package's one derivation. So
any change to scoot's own code changes the package hash and Smithay
compiles from scratch again, once per package (`scoot`,
`scoot-gpu-xwayland`, `scootctl`, `scootbg`, `scootbar`, `scootbar-demo`),
on every merge: Cachix then serves installs of an unchanged
revision but saves CI nothing. Moving the Rust builds to crane
(`buildDepsOnly` → a dependency-artifacts derivation keyed on `Cargo.lock`
and the dependency sources, forks included) puts the compiled dependencies
in their own store path, which Cachix keeps and every build reuses until
`Cargo.lock` or a fork rev changes (rare by design: `docs/forks.md`).

## What to do

Care points: scootbar's Cargo-feature `.override` (different features,
different dependency builds), the three compositor packages sharing one
dependency set, `cargoLock.outputHashes` for the git deps, and every
`checks.*` and `nix-build.yml` `--version` line still passing. Measure a
code-only rebuild before and after.

Once crane lands, reconsider building `scoot-gpu` and `scoot-xwayland` in
`nix-build.yml`: the header there explains they were left out because each
main-push build pays a full Smithay rebuild, which is exactly the cost
crane removes. (The full build `scoot-gpu-xwayland` is already built and
cached; it is the two in-between variants that wait on this.)

## Not in this ticket

The Cachix cache and FlakeHub publish themselves
([nix-publishing](../resolved/nix-publishing-done.md)).
