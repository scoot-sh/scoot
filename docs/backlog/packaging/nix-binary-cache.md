---
title: "A Nix binary cache for users, so `nix run` does not compile Smithay"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# A Nix binary cache

Filed 2026-09-29. Serves **daily-drive**: trying scoot or the bar is one
line only if it does not first compile a compositor. `nix-build.yml` measured a
cold build of two packages at 5m10s on four vCPUs, and the dev shell costs
~35s to realise.

## Not a reversal of an earlier decision

`ci.yml` records that CI keeps **no Nix store cache**, measured: 
`magic-nix-cache-action` now needs a FlakeHub account, failed to authenticate,
and cost time. That was about speeding CI. This entry is about a **substituter
for users**. CI need not pull from it; it only needs somewhere to push the
builds it already does.

## What to decide

- **Where**: Cachix (hosted; check current terms for open source), Attic
  (self-hosted over S3-compatible storage), Garnix (builds and caches flakes;
  check terms), or a plain bucket with `nix copy` and a signing key
  (least lock-in, most to run). FlakeHub Cache is the route CI dropped.
  Compare cost, trust model, secrets held in CI, `aarch64-linux` availability
  and how a user opts in.
- **What to push**: from `main` and release tags only, built by CI in a clean
  environment: `packages` for `scoot`, `scootctl`, `scootbg`,
  [`scootbar`](../../scootbar/backlog/nix-package.md) (and `scoot-gpu`, which CI
  skips today because of its cost), and the dev shell for contributors. Systems:
  `x86_64-linux` and `aarch64-linux`. Never push builds of fork pull requests: a
  poisoned cache entry is a supply-chain hole.
- **Trust**: a substituter is trusted with binaries. Publish the public key in
  the docs, keep the secret key only in CI secrets, write down the rotation
  plan, and say in the docs exactly what opting in trusts.
- **Opt-in**: flake `nixConfig` (`extra-substituters`,
  `extra-trusted-public-keys`), which prompts users; document the NixOS
  (`nix.settings`) and home-manager equivalents and the non-interactive case.
- **Cost control**: retention and garbage collection, and the size of what each
  main push adds (record closure sizes).

## Done when

A clean machine with the cache configured runs `nix run .#scootbar` without
compiling, the cold-versus-cached time is published, pushes come only from
trusted CI, and the key and rotation plan are documented.
