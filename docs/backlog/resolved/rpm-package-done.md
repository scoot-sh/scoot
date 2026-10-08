---
title: "RPM packages: Fedora and openSUSE-style `.rpm`s, and a COPR-style repository"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# RPM packages

Filed 2026-09-29. Serves **daily-drive**.

## Two levels, start with the first

1. **Repo-hosted RPMs** via `cargo-generate-rpm` (or a minimal spec file),
   attached to the release ([release-artifacts](../resolved/release-artifacts-done.md)) and
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

## Resolved (PR #526, 2026-10-08)

Level 1 landed; the COPR repository itself stays designed-and-documented
(a manual maintainer step, like the AUR push for the Arch packages).

- **Three spec files** (`packaging/rpm/scoot.spec`, `scootbg.spec`,
  `scootbar.spec`): hand-written minimal specs (the ticket allows
  `cargo-generate-rpm` or a minimal spec; the generated file would need
  the same hand edits). Built from the vendored-sources tarball
  (`scripts/vendor-release.sh`), offline (`CARGO_NET_OFFLINE=true`),
  default feature set. Trio tag for `scoot`/`scootbg` (lockstep),
  bar tag for `scootbar` (independent); `scoot` requires exactly
  `scootbg = Version-Release`. Session entry, bar user unit
  (`Restart=on-failure`, `%systemd_user_post`/`%systemd_user_preun`),
  `%license` set including the deny-generated inventory, example
  configs as `%doc`. `%check` runs only headless-safe probes.
- **CI** (`.github/workflows/rpm.yml`): `rpmbuild -ba` in Fedora 43 and
  openSUSE Tumbleweed containers (SUSE `%if` arms: `seatd-devel`,
  `libpixman-1-0-devel`), `rpmlint` on specs (clean) and RPMs (0 errors;
  `no-manual-page-for-binary` and `scootbg` `no-documentation`
  documented, plus a scoped `rpmlintrc` for openSUSE's `config`
  dictionary flag), install of the trio, `--version` on all three,
  `daemon --check` on the installed example, and the full headless
  smoke test (`rc=0`) against the installed Fedora binaries.
- **Verified locally first** in `fedora:43` and `opensuse/tumbleweed`
  containers: auto-Requires are exactly the five predicted sonames;
  installed sizes scoot 6.7 MB / scootbar 2.3 MB / scootbg 1.6 MB.
  Container truths recorded in `packaging/rpm/README.md`: the
  `tsflags=nodocs` / vendor-`excludedocs.conf` overrides, the SUSE
  `%doc` path (`/usr/share/doc/packages/`), the `busybox-gawk`
  conflict, `debug_package %{nil}` with explicit strip (vendored
  sources trip `brp-mangle-shebangs`), and the `DesktopNames=`
  validator notice Fedora's own session files share.
- **Not done**: no COPR account touched, nothing uploaded, no tag;
  `Source0` URLs 404 until the first release names its assets (only
  those lines change then). `x86_64` proven in CI; the specs carry no
  `ExclusiveArch`, and `aarch64` was proven locally.
