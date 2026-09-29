---
title: "Vendored sources, third-party licenses and advisory checks: what distro packaging needs first"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Vendoring, licenses and advisories

Filed 2026-09-29. Serves **daily-drive**. Every distro package
([arch](arch-package.md), [deb](deb-package.md), [rpm](rpm-package.md)) starts
here: distro builders are usually offline and want sources vendored, a
license inventory, and to know what is forked.

## What to build

- **Vendored release tarball**: `cargo vendor` output plus the source
  replacement snippet, produced by the release job
  ([release-artifacts](release-artifacts.md)), so a build needs no network.
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
