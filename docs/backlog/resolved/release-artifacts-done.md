---
title: "Release artifacts: tag-triggered builds, checksums, signatures, SBOM, changelogs"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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
  say which; the distro packages ([arch](arch-package-done.md), [deb](deb-package-done.md),
  [rpm](rpm-package.md)) build from the source tarball instead.
- **Changelog per package**, in the repo, written at release time; user-facing
  changes and config or protocol breaks called out.

## Done when

Pushing a tag produces a verifiable release for that package on both
architectures, the checksums and signature verify, and nothing is uploaded by
hand.

## Resolved (PR #498, 2026-10-07)

`.github/workflows/release.yml`: a tag push (`scoot-v0.2.0`,
`scootbar-v0.3.0`; a trio tag releases the trio's binaries) builds that
package only and publishes a GitHub Release -- vendored tarball
(`scripts/vendor-release.sh`), portable glibc binaries for
`x86_64-linux` (ubuntu-latest) and `aarch64-linux` (ubuntu-24.04-arm,
native, no emulation, no dependency on the maintainer's box),
`SHA256SUMS`, keyless build-provenance attestation (chosen over
minisign/GPG/cosign: verifiable with `gh attestation verify`, no
long-lived key in secrets), CycloneDX SBOM (cargo-cyclonedx 0.5.9 via
the flake's nixpkgs pin), per-package changelog as the notes -- plus a
`workflow_dispatch` dry run that builds everything and publishes
nothing. Gates (`scripts/release-resolve`, 13 unit tests in
`scripts/test_release_resolve.py`): tag shape, manifest match (trio
lockstep), tag on main, re-publish refused while incomplete releases
resume, so retries converge. Actions pinned by SHA, minimal per-job
permissions, no new secrets. Decisions with evidence: binaries come
from cargo in a `debian:bookworm-slim` container (glibc floor 2.36,
asserted) because the nix binary's `/nix/store` interpreter does not
run on a stock distro (portable build prints `--version` on bookworm,
trixie and fedora 43; the nix build fails there). The distro packages
(arch, deb, rpm) build from the source tarball instead, and are now
unblocked. Users verify on the site's releases page; maintainers cut
on the cut-a-release page. First dry run happens after merge (new
workflows cannot dispatch before they are on main); no tag was
created or pushed and no release published here.
