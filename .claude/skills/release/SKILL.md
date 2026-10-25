---
name: release
description: Cut, verify and propagate a scoot release. Merge the release PR (or hand-cut one for Nix/CI/docs-only changes), tag, watch release.yml, then bump scoot-iso and try-scoot and check both. Use when asked to release, cut a version, ship a tag, or bump the downstream repos.
---

# Releasing scoot

The full reference is `site/src/content/docs/reference/cut-a-release.md`. This
skill is the order of operations and the checks between steps. Stop at the
first red and report it; a red is a bug until a log proves otherwise.

## 1. Pick the path

- release-plz only counts changes inside crate directories. If a
  `chore: release vX.Y.Z` PR is open, use it. If the changes are Nix-only,
  CI-only or docs-only there is none: hand-cut (below).
- Hand-cut: bump `[workspace.package]` version, run `cargo update -w`,
  `scripts/sync-fuzz-locks`, `scripts/sync-packaging-versions`; add
  `## [X.Y.Z](compare-link) - date` directly under `## Unreleased` in
  `CHANGELOG.md`; open a normal PR (not a `release-plz-` branch); dispatch
  `deb.yml`, `rpm.yml`, `arch.yml` and `release.yml` (lint) on the branch.

## 2. Merge on a green exact head

- `gh pr checks <n>` and `gh pr view <n> --json headRefOid`: every check
  green, including the three packaging runs, on the head you will merge.
- Merge only with `gh pr merge <n> --squash --match-head-commit <sha>`.
- release-plz PRs tag and dispatch the release by themselves. For a
  hand-cut: `git tag -a vX.Y.Z -m "scoot vX.Y.Z" <merge sha>`, run
  `scripts/release-resolve vX.Y.Z --no-github` in a detached worktree, then
  `git push origin vX.Y.Z`.

## 3. Watch release.yml

- `gh run list --workflow release.yml --limit 3`, then watch the run for the
  tag with a background poll (the Bash tool blocks foreground `sleep`).
- Done means: tag gate, vendored tarball and SBOM, portable binaries on both
  architectures, publish, and verify on both architectures all `success`, and
  `gh release view vX.Y.Z --json assets` lists the assets (15 at v0.6.4).
- Also check `dispatch-downstreams` and `bump-downstreams.yml` (step 4).

## 4. Bump the downstreams

Run each repo's own workflow (no secret needed; it uses `GITHUB_TOKEN`):

```sh
gh workflow run bump-scoot.yml -R scoot-sh/scoot-iso -f tag=vX.Y.Z
gh workflow run bump-scoot.yml -R scoot-sh/try-scoot -f tag=vX.Y.Z
```

Watch each run (`gh run list -R scoot-sh/<repo> --workflow bump-scoot.yml`).
It runs scoot's `scripts/bump-downstream` from the tag, pushes
`ci/bump-scoot-vX.Y.Z`, opens or updates one PR, and dispatches `ci.yml` on
the branch. A run that finds the pin already current opens nothing and says
so. The scoot-side `bump-downstreams.yml` is a second route that needs
`DOWNSTREAM_BUMP_TOKEN`; ignore its red until that secret exists.

## 5. Validate and merge the downstream PRs

- The PR's own `pull_request` run waits at "action_required" (a
  `GITHUB_TOKEN` PR), and `gh pr checks` shows nothing. The real checks are
  the dispatched run's, attached to the head commit. Read them there:
  `gh api repos/scoot-sh/<repo>/commits/<head-sha>/check-runs -q '.check_runs[]|.name+" "+.conclusion'`,
  and make sure the run is `completed`, not still in progress.
- scoot-iso needs `check`, `kvm-probe`, `build-iso`, `qemu-test`,
  `docker-build` green. try-scoot needs `check` and both architectures of
  `build` and `build-selkies`; its dispatched run skips the `push` jobs
  (they only publish PR-tagged images) and `manifest`.
- Confirm the diff pins vX.Y.Z only (flake.nix and flake.lock, plus the
  `iso/target` flake and `tests/patch_consistency.py` for scoot-iso).
- Merge each with `gh pr merge <n> -R scoot-sh/<repo> --squash
  --match-head-commit <sha>`.

## 6. Close out

- `gh release view vX.Y.Z` is public, main's own workflow runs are green
  (`gh run list --branch main`), both downstream PRs show MERGED.
- Send `ntfy "Done: scoot vX.Y.Z released; scoot-iso and try-scoot bumped"`.
- Report the checks you did not run (for example, a bump workflow that has
  not yet run end to end).
