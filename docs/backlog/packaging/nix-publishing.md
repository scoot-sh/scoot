---
title: "Publish: binaries on Cachix, the flake on FlakeHub (no FlakeHub Cache)"
status: "open"
area: "packaging"
priority: "medium"
blocked: "the maintainer's Cachix cache, auth token secret and public key, and a FlakeHub org linked to scoot-sh"
---

# Publish: binaries on Cachix, the flake on FlakeHub (no FlakeHub Cache)

Filed 2026-10-03. Serves **daily-drive**: installing scoot from the flake
today compiles Smithay from scratch (about 5 minutes on 4 cores), and the
machines that matter most (the maintainer's Asahi M2, the aarch64 dev VM)
get no prebuilt anything. The maintainer's plan (2026-10-03): "Planning to
eventually do cachix and flakehub (without their flakehub cache)."

## Where things stand

- `.github/workflows/nix-build.yml` builds `scoot`, `scootctl`, `scootbg`,
  `scootbar` on x86_64-linux after each merge to `main`, and caches nothing.
  `ci.yml`'s note on `magic-nix-cache` explains why there is no store cache:
  that action started requiring a FlakeHub account, failed to authenticate,
  and cached nothing. Cachix takes a token instead.
- No aarch64 build in CI; no `nixConfig` substituter in `flake.nix`.

## What to do

**Cachix (binaries)**

- Push only from `main`, in `nix-build.yml` (`cachix/cachix-action`,
  `authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}`), never from PRs: unreviewed
  code must not reach a cache users trust. The action uploads only paths the
  job built, so nixpkgs paths already on cache.nixos.org are not duplicated.
- Add an aarch64-linux job on GitHub's arm runners (`ubuntu-24.04-arm`,
  free for public repos); consider building `scoot-gpu` there and on x86,
  since the cache removes the reason it was left out.
- `flake.nix` `nixConfig.extra-substituters` and
  `extra-trusted-public-keys`; `docs/nix.md`: how NixOS and home-manager
  users trust the cache. Add it to `vm/configuration.nix` and the Asahi box.
- Cachix-managed signing keys (no private key held by the project).

**Crane, so the cache helps CI and not only installs**

The packages are `buildRustPackage` derivations, which compile the whole
dependency graph (the `scoot-sh/smithay` fork and the `scoot-sh/wayland-rs`
`wayland-backend` patch included) inside each package's one derivation. So
any change to scoot's own code changes the package hash and Smithay
compiles from scratch again, once per package (`scoot`, `scootctl`,
`scootbg`), on every merge: Cachix then serves installs of an unchanged
revision but saves CI nothing. Moving the Rust builds to crane
(`buildDepsOnly` → a dependency-artifacts derivation keyed on `Cargo.lock`
and the dependency sources, forks included) puts the compiled dependencies
in their own store path, which Cachix keeps and every build reuses until
`Cargo.lock` or a fork rev changes (rare by design: `docs/forks.md`).
Care points: scootbar's Cargo-feature `.override` (different features,
different dependency builds), the three compositor packages sharing one
dependency set, `cargoLock.outputHashes` for the git deps, and every
`checks.*` and `nix-build.yml` `--version` line still passing. Measure a
code-only rebuild before and after.

**FlakeHub (the flake, versioned; no FlakeHub Cache)**

- `DeterminateSystems/flakehub-push` after the Cachix push in the same
  workflow, so a published flake version always has its binaries cached.
  Auth is GitHub OIDC (`permissions: id-token: write`), no secret.
- `include-output-paths: false` (that is the FlakeHub Cache feature, not used).
- **The flake has one version, but the packages version independently**
  ([independent-versioning](independent-versioning.md)): start with
  `rolling: true` (every merge to `main` is `0.1.<commit count>`), and add
  tagged releases on the compositor train's tags once versioning lands.
  Decide and document what a flake version promises.

## Blocked on (the maintainer, outward-facing)

1. A public Cachix cache (e.g. `scoot`), an auth token in the repo secret
   `CACHIX_AUTH_TOKEN`, and the cache's public key.
2. A FlakeHub account or org linked to `scoot-sh`.

## Not in this ticket

FlakeHub Cache (declined), crates.io publishing, distro packages.
