---
title: "Debian and Ubuntu packages: `.deb`s and a signed apt repository"
status: "open"
area: "packaging"
priority: "low"
blocked: "release-artifacts"
---

# Debian and Ubuntu packages

Filed 2026-09-29. Serves **daily-drive**.

## Two levels, start with the first

1. **Repo-hosted `.deb`s** built with `cargo-deb` (metadata in each crate's
   `Cargo.toml`), attached to the release
   ([release-artifacts](release-artifacts.md)) and published in a **signed apt
   repository** (`reprepro` or `aptly`). Dependencies come from
   `dpkg-shlibdeps` (`$auto`), so the list is derived, not remembered.
2. **A full `debian/` directory** only if inclusion in the Debian or Ubuntu
   archive is ever wanted: it needs offline builds from vendored sources
   ([vendoring-and-licenses](../resolved/vendoring-and-licenses-done.md)) and follows Debian
   Policy for the git-fork dependencies, which is real work. Not before there is
   demand.

## Details

- **Build in the oldest supported release's container** (a Debian stable and an
  Ubuntu LTS) so the glibc floor is stated. The Rust toolchain floor
  (`rust-version = "1.87"`) may be newer than those releases ship: check, and
  decide between installing a newer toolchain in the build container (fine for
  a repo-hosted package) or declining that release.
- Split packages per binary, matching the independent versions
  ([independent-versioning](../resolved/independent-versioning-done.md)); the coupled pair
  declares a versioned dependency.
- Files as in [arch-package](arch-package.md): the session entry, license
  and third-party inventory, no writes to user config, the bar's user unit, a
  clean removal.
- **CI**: build in the containers, lint with `lintian`, install, then run
  `scoot --version` and the headless smoke test against the installed files.
- `amd64` and `arm64`, via the same aarch64 build path as the releases.

## Done when

A stock Debian stable and Ubuntu LTS container installs from the signed
repository and passes the smoke test, with `lintian` clean or every warning
explained.
