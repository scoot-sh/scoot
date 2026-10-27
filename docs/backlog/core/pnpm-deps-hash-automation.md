---
title: "Automate the Nix pnpm-deps hash bump for dependabot site PRs"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Automate the Nix pnpm-deps hash bump for dependabot site PRs

Filed 2026-10-27. Serves **daily-drive** (a red docs-site build on every dependency bump blocks docs delivery; the fix is toil, not a decision).

## The gap

`nix/docs-site.nix` pins the vendored site dependencies with `pnpmDeps.hash` (`fetchPnpmDeps`, `fetcherVersion = 4`). The hash goes stale whenever `site/pnpm-lock.yaml` changes, so dependabot site PRs go red until a human rebases the hash by hand. Reported: PRs #734/#735 went red for this reason. Landed evidence: `fb137dcff` (`build(deps): bump zod and undici in the site (#756)`) bumps `zod 4.6.5 -> 4.6.7` and `undici 8.11.2 -> 8.12.3` in `site/pnpm-lock.yaml` and hand-updates the hash in `nix/docs-site.nix` (`sha256-evw2K0hmwjEtdw3dazZ7LH69/n2xsIjExcR+JbeGBws=` -> `sha256-lyM/gHhJhgPY3TUJB7dMYRfaXUa4L4oXRoPKMU3PQKo=`).

Two constraints, both verified in the tree:

- The repro is documented in `nix/docs-site.nix:23-24`: set the hash to `""`, run `nix build .#docs-site`, copy the `got: sha256-…` value back.
- The hash is Linux-specific: a Mac `nix build` computes a different value, so the canonical hash must come from a Linux build.

## What to do

- Either: a workflow that triggers on dependabot site PRs (lockfile changed, hash stale), builds `.#docs-site` on Linux, extracts the `got: sha256-…` value, and commits the one-line hash bump back to the PR branch.
- Or: a `scripts/` helper that does the Linux-build-plus-extract step plus a short note in `dev/` naming when to run it; keep the manual path one command.
- Either way: no unfixed-output weakening of the pin (the hash stays a fixed-output pin, never `""` or floating). Pin the automation with the smallest check that fails when the two files disagree (e.g. a CI job that rebuilds the hash on Linux and diffs it), and make sure the bot's own commit does not retrigger the workflow in a loop.

## Not in this ticket

Changing the vendoring approach (`fetchPnpmDeps` stays); site content, Astro/Starlight upgrades, or dependabot scheduling itself; the Mac-vs-Linux hash difference (accepted: Linux is canonical).
