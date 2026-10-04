---
title: "Nix: crane, so compiled dependencies are cached and CI rebuilds only scoot's crates"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
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

## Resolution (PR #420, 2026-10-04)

Landed as designed: crane v0.24.0 pinned by rev in `flake.nix`/`flake.lock`
(one lock node; `devenv.yaml`/vm revs untouched). Four `buildDepsOnly`
artifacts (base shared by `scoot`, `scootctl`, `scootbg` and every
`scootbar` `.override`, plus one per compositor feature combination --
`scootbar`'s flags gate only its own code, and its one dependency-bearing
flag `icon-image` names a `png` the base set already compiles).
`outputHashes` kept with the same values, re-keyed by Cargo.lock source
URL (crane's format); builds prove git vendoring works from a clean store
with no warnings. Every `checks.*` and `nix-build.yml` check passes
unchanged (`.override` interface and `cargoBuildFeatures`/
`cargoBuildNoDefaultFeatures` attrs kept as plain data).

Measured on the Asahi M2 (aarch64, 8 cores): code-only rebuild
(comment-only `.rs` in `scoot` and in `scootbar`) rebuilt 6 whole-graph
derivations before (186 s wall; live log shows Smithay recompiled) and
rebuilds 8 package derivations after while reusing all 4 dependency
derivations bit-for-bit (267 s wall; zero dependency recompiles in the
package logs). The dep artifacts are separate ~130 MB store paths the
workflow's Cachix daemon step uploads with every other built path.

`scoot-gpu` and `scoot-xwayland` joined the cached set in `nix-build.yml`:
under crane each costs one small first-party-only compile per code-only
merge, rebuilt fully only on lock/fork changes (rare by design).
