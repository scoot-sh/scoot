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
`Cargo.lock` or a fork rev changes (rare by design: `dev/forks.md`).

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

## Resolution (PR #420, 2026-10-04; fix round 2026-10-04)

Landed as designed: crane v0.24.0 pinned by rev in `flake.nix`/`flake.lock`
(one lock node; `devenv.yaml`/vm revs untouched). Five `buildDepsOnly`
artifacts (a flagless workspace-wide base set shared by `scootctl`,
`scootbg` and every `scootbar` `.override`, the compositor's own `-p scoot`
set with the `-lEGL` link flags, plus one per compositor feature
combination -- `scootbar`'s flags gate only its own code, and its one
dependency-bearing flag `icon-image` names a `png` the base set already
compiles). `outputHashes` kept with the same values, re-keyed by Cargo.lock
source URL (crane's format); builds prove git vendoring works from a clean
store with no warnings. Every `checks.*` and `nix-build.yml` check passes
unchanged (`.override` interface and `cargoBuildFeatures`/
`cargoBuildNoDefaultFeatures` attrs kept as plain data).

Reuse is proven at cargo's own word, not just by wall time: with
`CARGO_LOG=cargo::core::compiler::fingerprint=debug` the pre-fix tree
recompiled units whose fingerprints were *missing* from the inherited
artifact (same metadata hash nowhere in the dep build) -- the dep
derivations were built without the packages' `RUSTFLAGS` (shown by
`nix derivation show`) and with a workspace-wide `-p` scope whose feature
unification differs from each `-p` package build. Store mtimes are not a
factor (all normalized to epoch+1). After matching flags and scope per
artifact, a code-only rebuild recompiles nothing but first-party crates
for the four compositor packages.

Measured on the Asahi M2 (aarch64, `--cores 4 --max-jobs 1`, fresh clones:
commit A builds everything so the store stands in for Cachix, commit B is
A plus a comment-only touch in `scoot` and `scootbar`): before, `scoot`
rebuilt 129 crates in 110 s and `scoot-gpu-xwayland` 136 in 132 s
(Smithay, `wayland-backend` and `wayland-sys` among them); after, each of
the four compositor packages rebuilds 4 first-party crates (81--96 s,
Smithay never recompiled, zero dep derivations rebuilt), while
`scootctl`/`scootbg`/`scootbar` rebuild 9--12 crates each (13--39 s: their
own crates plus the small subgraph whose unified features differ between
the workspace-wide base artifact and a `-p` build). Same-6-package walls:
323 s before, 257 s after; the shipped 8-package set costs 433 s for two
more prebuilt variants. The dep artifacts are ~420 MiB each in the store
(~122--140 MB compressed, ~690 MB per architecture per lock/fork change,
code-only merges push only the rebuilt packages). The dep derivations
rebuild only when manifests, the lock or a fork rev changes (rare by
design); a lock change itself still pays full dep compiles once.

`scoot-gpu` and `scoot-xwayland` joined the cached set in `nix-build.yml`:
under crane each costs one small first-party-only compile per code-only
merge (87--89 s measured), rebuilt fully only on lock/fork changes (rare
by design).
