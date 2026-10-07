---
title: Cut a release
description: "Maintainer task: version, tag, push, dry-run and recover the tag-triggered release workflow."
---

Releases are cut per package from `main`. The tool never tags and the
workflow never bumps: you move the manifests, you push the tags, and
the [release
workflow](https://github.com/scoot-sh/scoot/blob/main/.github/workflows/release.yml)
takes it from there. The trio `scoot`/`scootctl`/`scootbg` moves in
lockstep (tag each moving package); `scootbar` moves on its own. What
each version means is on the [versions](./versions.md) page.

## Cut one, step by step

Run these on an up-to-date `main`, and push nothing until the tags are
on:

```sh
scripts/version plan --verbose
```

Read what it would do: the bump per package and the tags it would
create. Then bump each moving crate's `version` in its `Cargo.toml` to
the planned number (the trio together, the bar on its own), and commit
without touching anything else:

```sh
scripts/version check --allow-ahead
```

`--allow-ahead` is the release commit's own state (manifests newer
than any tag); plain `check` is what CI runs, and it must pass before
and after the tag lands.

```sh
git commit -m "chore(scoot,scootbg): release 0.2.0"
git tag scoot-v0.2.0 scootctl-v0.2.0 scootbg-v0.2.0
git push origin main scoot-v0.2.0 scootctl-v0.2.0 scootbg-v0.2.0
```

The `chore` type never bumps, whatever its scope, so the release
commit itself moves nothing. Push the commit and the tags together:
each tag starts one workflow run, and each run refuses a tag whose
commit is not on main. For the bar alone the same shape holds with one
tag:

```sh
git commit -m "chore(scootbar): release 0.3.0"
git tag scootbar-v0.3.0
git push origin main scootbar-v0.3.0
```

## What the workflow does with the tag

Each run resolves its tag (`scripts/release-resolve`: shape,
manifest match, on main, not already published), stages the vendored
tarball and the SBOM once (the SBOM is the union of the shipped
binaries' locked dependencies, so a trio release covers both `scoot`
and `scootbg`), builds portable glibc binaries for
`x86_64-linux` and `aarch64-linux` (cargo `--locked --offline` from
that staged tree in a Debian 12 container with Rust 1.97.0: glibc floor 2.36), checks
that every binary starts on the bare base, assembles `SHA256SUMS`,
attests every file with keyless build provenance (verify with the
`--signer-workflow` pin on the [releases](./releases.md) page), writes the
per-package changelog (`scripts/version changelog`) as the release
notes, and publishes the GitHub Release. A trio tag builds `scoot` and
`scootbg` (`scootctl` is the client library inside `scoot msg` and
ships no binary); a bar tag builds `scootbar`.

## Dry-run before the first real tag

Dispatch the workflow by hand with dry-run on (the default). It runs
resolve, vendor, both arch builds, the SBOM, the checksums and the
changelog, then prints what it would publish and stops — no release and
no attestation (it does upload the run's own build artifacts, kept 7
days, which is how the build and publish jobs hand files over):

```sh
gh workflow run release --ref main -f tag=scootbar-v0.3.0 -f dry_run=true
```

A dry run with `dry_run=false` publishes for real; that is the escape
hatch for re-cutting after a deleted release, not the normal path.
New workflow files cannot be dispatched until they are on main, so the
first dry run always happens after merge.

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
TAG=scootbar-v0.3.0
gh release delete "$TAG" --yes
git push origin ":refs/tags/$TAG"
```

Local tags are untouched by either command; drop yours too if you
made one (`git tag -d "$TAG"`).

## What the maintainer sets up

Nothing. Release creation and provenance run on `GITHUB_TOKEN` alone:
there is no signing key in secrets and no environment to approve.
What the runs need from the platform is what they already use
elsewhere in this repo: GitHub-hosted runners with Docker (the arch
builders), and third-party actions allowed (every action is pinned by
commit SHA). The two pins a human bumps over time are the distro base
(`debian:bookworm-slim`: the glibc floor) and the rustup toolchain
(`1.97.0` in `.github/workflows/release.yml`: the portable binaries
are built with Rust 1.97.0) — the toolchain must stay new enough
to compile the tree (the workspace floor predates the let-chains in
it), and the workflow asserts the toolchain and the glibc it produces
on every run. Verify downloads with `gh attestation verify <file>
--owner scoot-sh --signer-workflow
scoot-sh/scoot/.github/workflows/release.yml` (see the
[releases](./releases.md) page for why the signer pin matters).
