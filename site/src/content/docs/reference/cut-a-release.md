---
title: Cut a release
description: "Maintainer task: merge the release PR (or cut the first tag by hand), dry-run, and recover the tag-triggered release workflow."
---

A release is one tag, `vX.Y.Z`, that releases every binary at the
repository's one version. You choose the version in a release pull request
and merge it. The tag that merge pushes starts the
[release workflow](https://github.com/scoot-sh/scoot/blob/main/.github/workflows/release.yml),
which builds and publishes everything. What a version means, and how bumps
are proposed, is on the [versions](./versions.md) page and in
`docs/versioning.md`.

## Set up once: the release token

release-plz needs a token that can start other workflows (a tag pushed
with the default `GITHUB_TOKEN` starts none, so no release would build).
Create a fine-grained personal access token with:

- **Repository access:** only `scoot-sh/scoot`.
- **Repository permissions:** Contents: read and write; Pull requests:
  read and write. (Metadata: read is added automatically.)

Store it as an Actions secret named `RELEASE_PLZ_TOKEN`:

```sh
gh secret set RELEASE_PLZ_TOKEN --repo scoot-sh/scoot
```

The token's account authors the release PR. A GitHub App token with the
same two permissions works too. Until the secret exists, the
`release-plz` workflow posts a notice on every push and does nothing.

## Cut the first release (v0.2.0) by hand

The first tag is **v0.2.0**, never a 0.1.x: FlakeHub already published
rolling `0.1.*` versions, and a 0.1 tag would sort below them. The
workspace already reads `0.2.0`. release-plz waits for this first tag, so
cut it yourself, once.

First, a small PR that gives the release its changelog heading. In
`CHANGELOG.md`, add the version heading directly under the Unreleased
heading, so everything written so far becomes v0.2.0's notes:

```text
## Unreleased

## [0.2.0] - 2026-10-14

### The built-in cursors are Mocu (behavior change)
...
```

Merge it, then tag that merge commit on an up-to-date `main` and push the
tag:

```sh
git switch main && git pull --ff-only
scripts/workspace-version          # must print 0.2.0
git tag -a v0.2.0 -m "scoot v0.2.0"
scripts/release-resolve v0.2.0 --no-github   # the workflow's own gate, locally
git push origin v0.2.0
```

`release-resolve` refuses a tag whose version is not the workspace
version, or that is not on main. The push starts the release workflow;
watch its first run (the next sections say what it does and how to
recover).

## Every release after that

On every push to main, release-plz keeps one PR open titled
`chore: release vX.Y.Z`. It bumps `[workspace.package] version` and
writes the version's section in `CHANGELOG.md`: a list generated from
the commits since the last tag, grouped by component, above the hand
notes written under Unreleased.

1. Read the PR. The proposed version follows the commits (below 1.0: a
   breaking change bumps the minor, anything else the patch). To release
   a different version, edit `version` under `[workspace.package]` in
   `Cargo.toml` and the heading in `CHANGELOG.md` on the PR branch.
2. On the same branch, bump the distro packaging to that version:
   `pkgver` in the three Arch release PKGBUILDs (and their `.SRCINFO`),
   `Version` and a `%changelog` entry in the three RPM specs. The Arch
   and RPM workflows run on the release tag and fail when these lag the
   workspace version (`packaging/arch/README.md`,
   `packaging/rpm/README.md`).
3. Tidy the changelog section if it needs it, and let CI go green.
4. Merge. release-plz pushes `vX.Y.Z`, and the release workflow builds
   and publishes it.

## What the workflow does with the tag

The run resolves the tag (`scripts/release-resolve`: the `vX.Y.Z` shape,
the workspace version at the tagged commit, every crate inheriting it,
on main, not already published), stages the vendored tarball and the
SBOM once (the union of every shipped binary's locked dependencies),
builds `scoot`, `scootbg` and `scootbar` as portable glibc binaries for
`x86_64-linux` and `aarch64-linux` (cargo `--locked --offline` from
that staged tree in a Debian 12 container with Rust 1.97.0: glibc floor
2.36), checks that every binary starts on the bare base, assembles
`SHA256SUMS`, attests every file with keyless build provenance (verify
with the `--signer-workflow` pin on the [releases](./releases.md) page),
takes the version's `CHANGELOG.md` section as the release notes
(`scripts/release-notes`), and publishes the GitHub Release. `scootctl`
is the client library inside `scoot msg` and ships no binary of its own.

## Dry-run a tag

Dispatch the workflow by hand with dry-run on (the default) against a
tag that already exists on main. It runs resolve, vendor, both arch
builds, the SBOM, the checksums and the notes, then prints what it would
publish and stops — no release and no attestation (it does upload the
run's own build artifacts, kept 7 days, which is how the build and
publish jobs hand files over):

```sh
gh workflow run release --ref main -f tag=v0.2.0 -f dry_run=true
```

A dry run with `dry_run=false` publishes for real; that is the escape
hatch for re-cutting after a deleted release, not the normal path.

A dry run exercises the whole artifact round trip (vendor, build,
download, assemble, checksums) but **not** the attestation or the
release creation: those first run for real on the first tag. Expect to
watch that first run, and if it fails after the build, use the recovery
steps below before anyone downloads anything.

## When a run fails

> **Symptom:** the workflow went red on a tag push.

Re-run the failed run (the run's page, "Re-run jobs"). The gates
refuse a complete release but resume an incomplete one, re-uploading
only what is missing, so a retry converges instead of duplicating.

> **Symptom:** the published release is wrong (bad notes, wrong files)
> and a retry refuses with "already published".

Delete the release and its tag, then re-cut from the same commit:

```sh
TAG=v0.2.0
gh release delete "$TAG" --yes
git push origin ":refs/tags/$TAG"
```

Local tags are untouched by either command; drop yours too if you
made one (`git tag -d "$TAG"`).

> **Symptom:** a release PR merged but no tag appeared.

The `release-plz` workflow's "Tag a merged release PR" job ran without
the token (its notice says so) or failed. Set `RELEASE_PLZ_TOKEN` and
re-run that job, or push the tag by hand as for the first release.

## What else the runs need

Release creation and provenance run on `GITHUB_TOKEN`: there is no
signing key in secrets and no environment to approve. What the runs need
from the platform is what they already use elsewhere in this repo:
GitHub-hosted runners with Docker (the arch builders), and third-party
actions allowed (every action is pinned by commit SHA). The two pins a
human bumps over time are the distro base (`debian:bookworm-slim`: the
glibc floor) and the rustup toolchain (`1.97.0` in
`.github/workflows/release.yml`: the portable binaries are built with
Rust 1.97.0) — the toolchain must stay new enough to compile the tree
(the workspace floor predates the let-chains in it), and the workflow
asserts the toolchain and the glibc it produces on every run. Verify
downloads with `gh attestation verify <file> --owner scoot-sh
--signer-workflow scoot-sh/scoot/.github/workflows/release.yml` (see the
[releases](./releases.md) page for why the signer pin matters).
