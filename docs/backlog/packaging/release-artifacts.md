---
title: "Release artifacts: tag-triggered builds, checksums, signatures, SBOM, changelogs"
status: "open"
area: "packaging"
priority: "medium"
blocked: "independent-versioning"
---

# Release artifacts

Filed 2026-09-29. Serves **daily-drive**. Today there are no tags and no
releases; installs are from source or Nix.

## What to build

- A **release workflow triggered by a package tag** (`scootbar-v0.2.0`),
  no manual uploads, with a dry-run mode. It builds only that package.
- **Per release**: a source tarball with vendored dependencies
  ([vendoring-and-licenses](../resolved/vendoring-and-licenses-done.md)); prebuilt binaries for
  `x86_64-linux` and `aarch64-linux`; `SHA256SUMS`; signatures (options: minisign,
  GPG, or keyless signing and build-provenance attestation through GitHub;
  decide by who can verify and key-management cost); an SBOM
  (`cargo-cyclonedx`); the package's changelog section as the release notes.
- **Dynamic glibc binaries** (libseat, libinput and libxkbcommon rule out a
  static build): build on the oldest supported base so the glibc floor is
  stated, not accidental.
- **aarch64 builds**: CI has only `ubuntu-latest` and `macos-latest` today. Options to
  measure: a GitHub-hosted arm runner if available to this repo, emulation
  (slow for a Smithay build), or a Nix cross/remote build. The maintainer's own
  aarch64 machine is a NixOS box, so this must not depend on it being on.
- **One reference build**: decide whether release binaries come from `nix build`
  (bit-for-bit what Nix users get) or from `cargo` in a distro container, and
  say which; the distro packages ([arch](arch-package.md), [deb](deb-package.md),
  [rpm](rpm-package.md)) build from the source tarball instead.
- **Changelog per package**, in the repo, written at release time; user-facing
  changes and config or protocol breaks called out.

## Done when

Pushing a tag produces a verifiable release for that package on both
architectures, the checksums and signature verify, and nothing is uploaded by
hand.
