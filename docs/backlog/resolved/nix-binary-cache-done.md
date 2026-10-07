---
title: "A Nix binary cache for users, so `nix run` does not compile Smithay"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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
  [`scootbar`](../../scootbar/backlog/resolved/nix-package-done.md) (and `scoot-gpu`, which CI
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

## Resolution (2026-10-07, PR #499)

Met, verified with evidence rather than assumed. The CI side (`#410`) was
already built; what was missing was the proof and the docs, which this adds.

- **Clean machine, no compile.** Asahi M2, fresh empty store
  (`nix --store ~/fx/cache-470d1ea3/store run
  github:scoot-sh/scoot#scootbar --accept-flake-config -- --version`,
  flake resolved to main `0c27dc64`): prints `scootbar 0.1.0` in 14.6 s
  wall, 7 paths fetched (1.1 MiB download, 58.6 MiB unpacked), the bar
  itself copied from `https://scoot-sh.cachix.org`. Nothing compiled.
- **What the cache serves** (read-only `narinfo` checks against current
  main store paths): all seven installable outputs HIT on both
  `x86_64-linux` and `aarch64-linux` (`scoot`, `scoot-gpu`,
  `scoot-xwayland`, `scoot-gpu-xwayland`, `scootbg`, `scootbar`,
  `scootbar-demo` — the last a sub-KB wrapper over a cached `scootbar`).
  `scoot-gpu`, which this entry says CI skipped for cost, is now built
  and pushed (crane made each variant one small compile). Not cached, by
  recorded decision: the docs site (built where it deploys), the dev
  shell (realised from nixpkgs plus the toolchain — see deviation), and
  macOS builds (macOS CI is check-only, Darwin store paths 404).
- **Cold vs cached, published** on `site/src/content/docs/start/binary-cache.md`:
  ticket-original 5m10s (two packages, 4 vCPUs); CI code-change push
  (run `37587122304`) ~21/19 min per arch with 317 deps substituted
  (775.3 MiB) before compiling the changed crates; fully-cached push
  (run `37604984779`) 58–76 s per job; M2 single-package fetch 14.6 s.
  Scootbar closure 5 paths / 48.5 MiB, binary 2,650,936 bytes stripped.
- **Trusted CI only.** `nix-build.yml` triggers on `push` to `main` and
  `workflow_dispatch` — never `pull_request`, so fork runs never reach
  it. The authenticated Cachix step is gated to `push`/`refs/heads/main`
  and is the sole recipient of `secrets.CACHIX_AUTH_TOKEN` (org secret,
  repo-scoped); dispatches take a token-less pull-only step. Signing is
  Cachix-managed; the project holds no private key. Publish to FlakeHub
  is flake-only (`include-output-paths: false`) over OIDC after both
  arch builds. Residual writer-trust boundary recorded on the page.
- **Key and rotation documented.** Public key
  `scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo=`
  matches `flake.nix` `nixConfig` and the read-only
  `app.cachix.org/api/v1/cache/scoot-sh` response. Rotation runbook
  (token + signing key), retention/size notes, and per-shape opt-in
  (NixOS `nix.settings`, home-manager `nix.settings`, `nix.conf`,
  `--accept-flake-config` with the trusted-users caveat) are on the new
  page, linked both ways from Install and the desktop flake setup.

Deviations recorded honestly: the **dev shell is not pushed** (this
entry asked for it) — a dev shell is not a package output CI builds, so
contributors still realise it (~35 s per the original note, not
re-measured); the page says so. `scootctl` in "What to push" is gone as
a package (`scoot msg` since `#464`). No rotation has ever been
exercised — the runbook is written, not yet drilled. Cache totals are
not publicly visible, so retention is stated as Cachix-plan GC plus
measured per-push sizes, not a dashboard number.
