---
title: Releases
description: "Get a scoot release, know which file is which, and verify it: checksums, provenance, SBOM."
---

Every release is one tag, `vX.Y.Z` (`v0.2.1`), and holds every binary
at that one version: the compositor with `scoot msg`, the wallpaper
daemon, the bar and the launcher. The [release workflow](#where-releases-come-from)
builds and publishes it, nothing by hand. If you are deciding whether
versions work together, read [Versions and
compatibility](./versions.md) first.

## Which file to download

A release holds one set of files per architecture (`x86_64-linux`,
`aarch64-linux`), plus the sources everything was built from:

| File | What it is |
|---|---|
| `scoot-<version>-<arch>` | The compositor (plus the `scoot msg` client, the only client) |
| `scootbg-<version>-<arch>` | The wallpaper daemon |
| `scootbar-<version>-<arch>` | The status bar |
| `scootlaunch-<version>-<arch>` | The application launcher |
| `scoot-<version>-vendored.tar.gz` | The exact tagged tree plus `vendor/` (every dependency), `.cargo/config.toml`, `THIRD-PARTY-LICENSES` and `OFFLINE-BUILD.txt` |
| `sbom-scoot-<version>.cyclonedx.json` | The locked dependencies of every shipped binary (their union), CycloneDX 1.3 |
| `SHA256SUMS` | The checksum of every file above |

The binaries are the default feature build: one binary that scans out
from the GPU on real hardware and renders on the CPU everywhere else
(the GPU scanout tier is compiled in and reaches the GPU libraries at
run time via `dlopen`, so the same binary starts with and without a GPU
stack installed), with `xwayland` compiled in (needs the `Xwayland`
binary on `PATH` at run time to drive X11 apps). They are built with cargo from the
vendored tree on Debian 12 (glibc 2.36) with Rust 1.97.0 (the rustup
pin in `release.yml`), so they run on any glibc distro at or past that
floor. For a CPU-only build with no GPU or X11 code at all, build from
the vendored tarball ([package scoot offline](./packaging.md)) or take
the Nix packages.

The easy way to get them: the [install
script](../../start/install/#use-it-every-day) picks the right files
for your machine, checks them against `SHA256SUMS`, and installs the
session files beside them.

## Tagged releases only

Releases are tagged, and the [install
script](../../start/install/#use-it-every-day) installs them: by
default the newest tag, or the version `--version` names. When no
tagged release exists yet the script fails instead of installing
`main` — it never follows a moving build. To run `main`, use the Nix
flake: every merge to `main` already lands tested binaries in the
Cachix cache with the flake on FlakeHub (rolling release) — see
[Install](../start/install.md). Verify a file with
`--signer-workflow scoot-sh/scoot/.github/workflows/release.yml` in
the command below.

A build that is not a release says how far past one it is: `scoot
--version` prints `scoot 0.2.1+14.g615608c (ipc protocol 11)`. The Nix
flake's rolling release is numbered `0.3.<commits on main>`, so it sorts
above every 0.2.x tag.

## Run them on your distro

The bar and the wallpaper daemon need nothing but a C library. The
launcher needs `libxkbcommon` beside it. The
compositor needs five system libraries beside it — the `ldd`-derived
list, not a guess:

| Distro | Install (compositor only) |
|---|---|
| Debian 12 / Ubuntu 24.04 and newer | `apt install libseat1 libinput10 libxkbcommon0 libudev1 libpixman-1-0` |
| Fedora 43 and newer | `dnf install libseat libinput libxkbcommon systemd-libs pixman` |
| Arch | `pacman -S seatd libinput libxkbcommon systemd-libs pixman` |

Older releases work if they provide the same sonames
(`libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`); a musl distro (Alpine) is
not covered — these are glibc binaries. Check what you have before
anything else:

```sh
chmod +x scoot-0.2.1-x86_64-linux   # release downloads carry no executable bit
./scoot-0.2.1-x86_64-linux --version
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
rotate. You need the `gh` CLI (the flag spelling below is
checked against `gh attestation verify --help` at gh 2.92.0 and
2.100.0; `ubuntu-latest` runners provide a recent `gh`):

```sh
gh attestation verify scoot-0.2.1-x86_64-linux --owner scoot-sh --signer-workflow scoot-sh/scoot/.github/workflows/release.yml
```

Pinning `--signer-workflow` matters: `--owner` alone accepts a
provenance attestation minted by any workflow in the org.

A verified file prints who built it (the release workflow, on the
tag's commit) and what went in. The SBOM is plain JSON — read it with `jq`:

```sh
jq '.components | length' sbom-scoot-0.2.1.cyclonedx.json
jq -r '.components[].name' sbom-scoot-0.2.1.cyclonedx.json | sort | head
```

## Where releases come from

A tag on `main` starts the [release
workflow](https://github.com/scoot-sh/scoot/blob/main/.github/workflows/release.yml):
it refuses a tag that disagrees with the workspace version, a tag not
on main, and a re-publish of an existing release; builds the tarball,
both architectures, the SBOM and the checksums; and publishes the GitHub
Release with that version's changelog section as its notes. Nix users can
skip the verification section above: every merge to main already lands tested
binaries in the Cachix cache with the flake on FlakeHub — see
[Install](../start/install.md). Cutting a release yourself is a
maintainer task, described on the [cut a release](./cut-a-release.md)
page.
