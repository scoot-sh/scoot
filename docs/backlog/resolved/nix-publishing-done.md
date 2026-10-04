---
title: "Publish: binaries on Cachix, the flake on FlakeHub (no FlakeHub Cache)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
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
  free for public repos); the cached set is `scoot`, `scoot-gpu-xwayland`,
  `scootctl`, `scootbg`, `scootbar` and `scootbar-demo` on both arches --
  the in-between variants `scoot-gpu` and `scoot-xwayland` stay out until
  crane (each would cost another full Smithay compile per push per arch).
- `flake.nix` `nixConfig.extra-substituters` and
  `extra-trusted-public-keys`; `docs/nix.md`: how NixOS and home-manager
  users trust the cache. Add it to `vm/configuration.nix` and the Asahi box.
- Cachix-managed signing keys (no private key held by the project).

**Crane, so the cache helps CI and not only installs** — split out to
[nix-crane](nix-crane.md).

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

## Post-merge checklist (the accounts exist; the first `main` run proves it)

The provisions are in place: the public `scoot-sh` Cachix cache, the
`CACHIX_AUTH_TOKEN` org secret scoped to this repo, and the FlakeHub org
linked to `scoot-sh`. What remains is the first push-to-`main` run, which
must show:

1. Both `build` legs green (`ubuntu-latest`, `ubuntu-24.04-arm`), the
   Cachix step uploading (not skipping) the six packages' paths per arch.
2. The `publish` job green after both legs.
3. The flake visible at `https://flakehub.com/flake/scoot-sh/scoot` with a
   `0.1.*` rolling release matching the merge commit (`fh list versions
   scoot-sh/scoot "0.1.*"`).
4. A consumer `nix build` with the documented trust config substitutes
   (not compiles) `.#scoot` on both architectures.
5. The Asahi box cache still needs adding by hand (out of reach of CI).

## Not in this ticket

FlakeHub Cache (declined), crates.io publishing, distro packages.

## Resolution (2026-10-04, PR #410, merge `b49f2ee9b`)

Landed: Cachix (`scoot-sh`) pushes from `main` only, on x86_64-linux and
aarch64-linux (`ubuntu-24.04-arm`), of `scoot`, `scoot-gpu-xwayland`,
`scootctl`, `scootbg`, `scootbar`, `scootbar-demo`; the token reaches only
the step gated to a push to `main` (a `workflow_dispatch` run uses a
token-less, pull-only step: the review found that `cachix-action` installs
the token whatever `skipPush` says); the two publishing actions pinned by
SHA; FlakeHub publishes `scoot-sh/scoot` (rolling, public, no output
paths) after both builds pass; `flake.nix` `nixConfig` names the cache;
`docs/nix.md` covers trusting it and installing from FlakeHub; the dev VM's
config names the cache (applied by the maintainer). Crane moved to
[nix-crane](../packaging/nix-crane.md).

The first `main` run (37168786109) proved the rest:

- Both build legs and the publish job succeeded.
- The Cachix push logs show the six packages' outputs on both
  architectures (14 package paths, scootbar's test variants included).
- From the aarch64 dev VM: `nix eval` of `scootbar`'s out path at the merge
  commit, then `nix path-info --store https://scoot-sh.cachix.org` on it,
  found it in the cache.
- `nix flake metadata "https://flakehub.com/f/scoot-sh/scoot/0.1.*.tar.gz"`
  resolves to `0.1.1492+rev-b49f2ee9bd8eb1b14286ae1dacd5f7958e970198`.

Left by hand: adding the cache to the Asahi box's NixOS config, and
applying `vm/configuration.nix` on the dev VM.
