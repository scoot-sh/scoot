---
title: "RPM packages: Fedora and openSUSE-style `.rpm`s, and a COPR-style repository"
status: "open"
area: "packaging"
priority: "low"
blocked: "release-artifacts"
---

# RPM packages

Filed 2026-09-29. Serves **daily-drive**.

## Two levels, start with the first

1. **Repo-hosted RPMs** via `cargo-generate-rpm` (or a minimal spec file),
   attached to the release ([release-artifacts](release-artifacts.md)) and
   published through a hosted build service or a signed repository (Fedora's
   COPR is the obvious host; check its terms, chroots and architecture
   coverage rather than assume them). Requires are auto-generated from the
   binary.
2. **A distro-grade spec** (vendored sources, `%check`, Fedora's Rust packaging
   conventions) only if inclusion in a distribution is ever wanted; the git-fork
   dependencies make it real work
   ([vendoring-and-licenses](../resolved/vendoring-and-licenses-done.md)).

## Details

- **Build in `mock` or a container per target release**, so the glibc and
  toolchain floors are stated (the `rust-version = "1.87"` floor may be newer
  than some releases ship: check).
- Split subpackages per binary, matching
  [independent-versioning](../resolved/independent-versioning-done.md); the coupled pair
  declares versioned requirements.
- Files as in [arch-package](arch-package.md): the session entry, license and
  third-party inventory, no writes to user config, the bar's user unit, clean
  removal.
- **CI**: build, lint with `rpmlint`, install, run `scoot --version` and the
  headless smoke test against the installed files.
- `x86_64` and `aarch64`, via the same aarch64 build path as the releases.

## Done when

A clean Fedora container installs from the repository and passes the smoke
test, with `rpmlint` clean or every warning explained.
