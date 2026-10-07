---
title: Releases
description: "Get a scoot release, know which file is which, and verify it: checksums, provenance, SBOM."
---

Every release is cut from a package tag (`scoot-v0.2.0`,
`scootbar-v0.3.0`) by the [release workflow](#where-releases-come-from),
nothing by hand. The compositor and the wallpaper daemon release
together (one version line); the bar releases on its own. If you are
deciding whether versions work together, read [Versions and
compatibility](./versions.md) first.

## Which file to download

A release holds one set of files per architecture (`x86_64-linux`,
`aarch64-linux`), plus the sources everything was built from:

| File | What it is |
|---|---|
| `scoot-<version>-<arch>` | The compositor (plus the `scoot msg` client, the only client) |
| `scootbg-<version>-<arch>` | The wallpaper daemon (same version as `scoot`) |
| `scootbar-<version>-<arch>` | The status bar (only in bar releases; its version is independent) |
| `scoot-<commit>-vendored.tar.gz` | The exact tagged tree plus `vendor/` (every dependency), `.cargo/config.toml`, `THIRD-PARTY-LICENSES` and `OFFLINE-BUILD.txt` |
| `sbom-<package>-<version>.cyclonedx.json` | The locked dependencies of the shipped binary (union of both binaries for trio releases), CycloneDX 1.3 |
| `SHA256SUMS` | The checksum of every file above |

The binaries are the default feature build: the GPU-free compositor
that runs on a box with no GPU stack installed, and a bar that links
nothing beyond the C library. They are built with cargo from the
vendored tree on Debian 12 (glibc 2.36) with Rust 1.97.0 (the rustup
pin in `release.yml`; the workspace `rust-version` floor predates the
let-chains the tree uses), so they run on any glibc
distro at or past that floor. What they are not: the `gpu-scanout`
build (needs libgbm at run time) and the `xwayland` build (needs the
`Xwayland` binary on `PATH`). For those, build from the vendored
tarball ([package scoot offline](./packaging.md)) or take the Nix
packages, which cover all four combinations.

## Run them on your distro

The bar and the wallpaper daemon need nothing but a C library. The
compositor needs five system libraries beside it — the `ldd`-derived
list, not a guess:

| Distro | Install (compositor only) |
|---|---|
| Debian 12 / Ubuntu 24.04 and newer | `apt install libseat1 libinput10 libxkbcommon0 libudev1 libpixman-1-0` |
| Fedora 43 and newer | `dnf install libseat libinput libxkbcommon systemd-libs pixman` |
| Arch | `pacman -S libseat libinput libxkbcommon systemd-libs pixman` |

Older releases work if they provide the same sonames
(`libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`); a musl distro (Alpine) is
not covered — these are glibc binaries. Check what you have before
anything else:

```sh
./scoot-0.2.0-x86_64-linux --version
```

It prints one line and exits without starting anything. If a library
is missing, that command names it.

## Verify a download

> **Symptom:** you want to know the file you fetched is the file the
> release built.
>
> Check the checksum first (it catches a corrupt or swapped download),
> then the provenance (it ties the file to the public build that made
> it).

```sh
sha256sum -c SHA256SUMS
```

Every line must report `OK`. Then the provenance: each file carries a
keyless GitHub attestation — no signing key to fetch, nothing to
rotate. You need the `gh` CLI (recent: the flag spelling below is
checked against `gh attestation verify --help` at gh 2.92.0 and
2.100.0; `ubuntu-latest` runners provide a recent `gh`):

```sh
gh attestation verify scoot-0.2.0-x86_64-linux --owner scoot-sh --signer-workflow scoot-sh/scoot/.github/workflows/release.yml
```

Pinning `--signer-workflow` matters: `--owner` alone accepts a
provenance attestation minted by any workflow in the org.

A verified file prints who built it (the release workflow, on the
tag's commit) and what went in. The SBOM is plain JSON for the same
reason — read it with `jq`:

```sh
jq '.components | length' sbom-scoot-0.2.0.cyclonedx.json
jq -r '.components[].name' sbom-scoot-0.2.0.cyclonedx.json | sort | head
```

## Where releases come from

A tag on `main` starts the [release
workflow](https://github.com/scoot-sh/scoot/blob/main/.github/workflows/release.yml):
it refuses a tag that disagrees with its manifest, a tag not on main,
and a re-publish of an existing release; builds the tarball, both
architectures, the SBOM and the checksums; and publishes the GitHub
Release with the per-package changelog as its notes. Nix users can
ignore all of the above: every merge to main already lands tested
binaries in the Cachix cache with the flake on FlakeHub — see
[Install](../start/install.md). Cutting a release yourself is a
maintainer task, described on the [cut a release](./cut-a-release.md)
page.
