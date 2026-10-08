---
title: "Debian and Ubuntu packages: `.deb`s and a signed apt repository"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Debian and Ubuntu packages

Filed 2026-09-29. Serves **daily-drive**.

## Two levels, start with the first

1. **Repo-hosted `.deb`s** built with `cargo-deb` (metadata in each crate's
   `Cargo.toml`), attached to the release
   ([release-artifacts](../resolved/release-artifacts-done.md)) and published in a **signed apt
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
- Files as in [arch-package](../resolved/arch-package-done.md): the session entry, license
  and third-party inventory, no writes to user config, the bar's user unit, a
  clean removal.
- **CI**: build in the containers, lint with `lintian`, install, then run
  `scoot --version` and the headless smoke test against the installed files.
- `amd64` and `arm64`, via the same aarch64 build path as the releases.

## Done when

A stock Debian stable and Ubuntu LTS container installs from the signed
repository and passes the smoke test, with `lintian` clean or every warning
explained.

## Resolved (PR #528, 2026-10-08; no tags, releases, uploads or keys)

`packaging/deb/` plus `.github/workflows/deb.yml` (mirrors the
arch-package shape in `packaging/arch`, PR #517): three `.deb`s
(`scoot`, `scootbg`, `scootbar`) built with cargo `--release --locked
--offline` from the vendored tree in `debian:bookworm-slim` (glibc
floor 2.36, asserted) with the pinned rustup toolchain 1.97.0 (the
distro toolchains predate the 1.87 floor), one native build per
architecture. `Depends` comes out of `dpkg-shlibdeps -O` at build time
checked against the `ldd`-derived allow-list (`deb-meta.py`, unit
tested in `test_deb.py` including refusal cases); the coupled
`scoot`/`scootbg` pin renders from the same run. Installed files match
arch-package (session entry, bar user unit, licenses plus the
deny-generated inventory and a `changelog.Debian.gz`, no user-config
writes, no conffiles or maintainer scripts so removal is clean).
`lintian` reports no errors; the two standing warnings
(`no-manual-page`, `initial-upload-closes-no-bugs`) are documented in
the README, not overridden. The signed repository is designed
(`reprepro`, in the README) with only the no-secret piece shipped
(`make-repo-index.sh` builds the unsigned Packages/Release, proven by
installing from it); the key, host, suites and publishing stay
maintainer steps, nothing secret pre-created. Deliberate deviations:
plain `cargo` + `dpkg-deb` instead of the ticket's `cargo-deb` (no new
build dependency, no manifest entanglement, `$auto` kept literally),
and no `debian/` source package (the ticket's level 2, on demand).
CI installs from the unsigned index on bookworm (full proof:
`--version` × 3, bar `--check`, headless smoke test) and noble
(glibc-forward install). Proven on the Asahi M2 (arm64) except the
final compile-to-smoke chain: vendored staging, all-three release
compile, control rendering, the shlibdeps gate both ways, lintian,
index, install, purge-to-clean, desktop validation. CI has since
proven the whole chain green on both architectures (PR #528 head
`a2d0e006a`, deb workflow run 37759835767: `scoot_0.1.0-1_amd64.deb`
and `_arm64` siblings, Depends exactly the allow-list, lintian 0
errors with only the two documented warnings, install + smoke green
on bookworm and noble).
