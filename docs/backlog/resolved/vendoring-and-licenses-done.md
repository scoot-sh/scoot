---
title: "Vendored sources, third-party licenses and advisory checks: what distro packaging needs first"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Vendoring, licenses and advisories

Filed 2026-09-29. Serves **daily-drive**. Every distro package
([arch](arch-package-done.md), [deb](deb-package-done.md), [rpm](rpm-package.md)) starts
here: distro builders are usually offline and want sources vendored, a
license inventory, and to know what is forked.

## What to build

- **Vendored release tarball**: `cargo vendor` output plus the source
  replacement snippet, produced by the release job
  ([release-artifacts](release-artifacts-done.md)), so a build needs no network.
  The two git dependencies, the Smithay fork and the `wayland-backend` fork
  (`[patch.crates-io]` in `Cargo.toml`), must vendor and resolve correctly:
  that is the part most likely to break, so test an offline build from the
  tarball in CI.
- **A for-packagers document** (`docs/packaging.md`): the fork list and why
  (`docs/forks.md`), the pinned revs, what the optional `gpu-scanout` feature
  adds, and the system libraries each feature links, derived from `ldd` of a
  release build rather than written from memory. When the Smithay repin lands
  ([smithay-fork-repin](../core/smithay-fork-repin.md)) this gets simpler.
- **Third-party license inventory**: generate it (`cargo-about` or
  `cargo-deny`) into a file shipped in every artifact, and fail CI on a license
  outside an allow-list in `deny.toml`. scoot itself is MIT.
- **Advisories**: a RustSec check (`cargo-deny advisories`) on a schedule and on
  dependency changes, so a vulnerable dependency is a visible red, not a
  surprise.
- **Minimum Rust version**: the workspace sets `rust-version = "1.87"`. Record
  which current distro releases ship at least that, since it decides whether a
  distro can build from its own toolchain; check, do not assume.
- `--locked` everywhere; `Cargo.lock` is already committed.

## Done when

The release tarball builds offline in a clean container, the license file is
generated and gated, and `docs/packaging.md` lets a packager work without
asking.

## Resolution (2026-10-07, PR #491, no tags or releases minted)

Built everything in this ticket that does not depend on tags/releases
existing; where the ticket needs the release job, the work is a callable
entry point `release-artifacts` uses later, and that is stated at each item.

- **Vendored tarball path**: `scripts/vendor-release.sh` stages
  `git archive HEAD` plus `cargo vendor` (both scoot-sh git forks vendor
  and resolve: Smithay `fdf424d6`, wayland-backend `70f81e00`, with the
  source-replacement snippet in `.cargo/config.toml`), generates
  `THIRD-PARTY-LICENSES` and `OFFLINE-BUILD.txt`, and tars with sha256.
  `--check-offline` builds the staged workspace under an empty
  `CARGO_HOME` with `CARGO_NET_OFFLINE` first. Proven on the Asahi M2:
  full workspace offline build exit 0 (144 crates, all three binaries),
  script run exit 0 with a 63.5 MB tarball. CI's vendor job
  (`.github/workflows/packaging.yml`) runs the script with
  `--check-offline` on every dependency change, so the release job later
  calls the same entry point (`release-artifacts` stays open for tags,
  signatures, SBOM, changelogs and the aarch64/oldest-glibc questions).
- **Packager docs**: `site/src/content/docs/reference/packaging.md`
  ("Package scoot offline", in the sidebar and the reference hub) instead
  of the ticket's `docs/packaging.md`, per the 2026-10-05 site move (user
  pages live on the site; contributor material stays in `docs/`). Fork
  list with pinned revs and one-line reasons, what `gpu-scanout` adds,
  `ldd`-derived system libraries per feature set from release builds
  (default 6.8 MB, gpu-scanout +libgbm/+libdrm 7.4 MB, xwayland no new
  soname 7.9 MB), the `deny.toml` policy summary, and symptoms.
- **License inventory and gate**: cargo-deny (not cargo-about: one
  maintained tool covers advisories too, single small binary, license
  check fully offline from `Cargo.lock`; 0.20.2 via the flake's nixpkgs,
  now also in the dev shell). `deny.toml` allow-lists MIT, Apache-2.0,
  ISC, BSD-3-Clause, Unicode-3.0, Zlib, Unlicense, 0BSD and fails closed
  on anything else. The inventory is `cargo deny list` output shipped as
  `THIRD-PARTY-LICENSES` in every tarball. Negative control: removing MIT
  fails `cargo deny check licenses`, restoring passes.
- **Advisories**: `cargo deny check advisories` on dependency changes
  and a weekly Monday schedule (separate workflow, since a schedule on
  `ci.yml` would run the whole matrix weekly). Two accepted risks
  recorded in `deny.toml` with reasons: RUSTSEC-2024-0436 (paste,
  compile-time only via pixman) and RUSTSEC-2026-0192 (ttf-parser, local
  fonts only via ab_glyph); no vulnerabilities or yanked crates.
- **Minimum Rust version**: per-distro table on the site page, checked
  2026-10-07 against each distro's own metadata. Debian 13 (1.85.1) and
  Ubuntu through 25.10 (1.75 / 1.85.1) cannot build from their distro
  toolchain; Debian sid, Ubuntu 26.04, Fedora 43, Arch, Alpine edge and
  openSUSE Tumbleweed can.
- **Dependabot triage**: the one open alert (postcss-selector-parser,
  moderate, transitive via `@expressive-code/core` -> `postcss-nested`
  6.x in the docs site) parses only our own sources at site build time
  and needs upstream to move to parser 7.x, so it is not a plain version
  bump; left for the maintainer, matching the two prior same-shape
  dismissals. Never opened anything upstream.
