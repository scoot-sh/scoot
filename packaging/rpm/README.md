# RPM packages (Fedora and openSUSE)

Four spec files, one per shipped binary, built from the vendored-sources
tarball (`scripts/vendor-release.sh`: the tagged tree plus `vendor/`,
`.cargo/config.toml` and `THIRD-PARTY-LICENSES`, so the build is offline).
Nothing here is published anywhere: there is no COPR project, no repo, no
tag, no release in CI or out of it.

| Spec | Package | Source tag | Version follows |
|---|---|---|---|
| `scoot.spec` | `scoot` | `v$Version` | the workspace version (`Cargo.toml`) |
| `scootbg.spec` | `scootbg` | `v$Version` | the same |
| `scootbar.spec` | `scootbar` | `v$Version` | the same |
| `scootlaunch.spec` | `scootlaunch` | `v$Version` | the same |

One tag releases every binary at the repository's one version
(docs/versioning.md), so all four specs carry the same `Version` and
the same release tarball. There is no `scootctl` package: it is the
client library inside the `scoot` binary (`scoot msg`). `scoot` requires exactly `scootbg=$Version-$Release`:
the coupled pair installs at one version, and the bar and the launcher
require no compositor package at all (standard Wayland protocols only).

Why a hand-written spec and not `cargo-generate-rpm`: the generated file would need the same hand edits anyway
(the session entry, the session user units, the portal selection, the
bar's user unit, the license set including the
deny-generated inventory, the vendored offline build, the versioned
`scootbg` requirement). The spec is the whole package definition; what
`cargo-generate-rpm` would contribute (the file list, the soname
requirements) is four binaries RPM derives on its own.

## Dependencies, from data

`BuildRequires` is the `ldd`-derived list from the site's packaging page
(`site/src/content/docs/reference/packaging.md`), not memory: the
compositor links `libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`, the launcher links
`libxkbcommon.so.0` for its keyboard (plus `libm.so.6` through the SVG
icon stack, which rides inside the C library package everywhere, so no
extra requirement names it), and the wallpaper daemon and the
bar link nothing past the C library. Runtime requirements for those
sonames are auto-generated from the binaries at build time, so only the
`-devel` names below are written in the specs. Verified 2026-10-08
against each distro's own metadata (`dnf repoquery --whatprovides` on
Fedora 43, `zypper what-provides` on Tumbleweed); re-run those two
commands at packaging time, names move:

| Soname | Fedora 43 (dnf) | openSUSE Tumbleweed (zypper) |
|---|---|---|
| `libinput.so.10` | `libinput` / `libinput-devel` | `libinput10` / `libinput-devel` |
| `libseat.so.1` | `libseat` / `libseat-devel` | `libseat1` / `seatd-devel` |
| `libudev.so.1` | `systemd-libs` / `systemd-devel` | `libudev1` (minimal images: `libudev-mini1`) / `systemd-devel` |
| `libpixman-1.so.0` | `pixman` / `pixman-devel` | `libpixman-1-0` / `libpixman-1-0-devel` |
| `libxkbcommon.so.0` | `libxkbcommon` / `libxkbcommon-devel` | `libxkbcommon0` / `libxkbcommon-devel` |
| `libgbm.so.1` / `libdrm.so.2` (run-time detected, never linked) | `mesa-libgbm` / `libdrm` | not probed (see below) |

Notes:

- The GPU libraries stay out of the link closure (the single build
  reaches them at run time through the in-tree stub): `mesa-libgbm` /
  `libdrm` are `Recommends` on Fedora (auto-detected, never required),
  and rebuilding without the tier at all (`--no-default-features`) is a
  local `rpmbuild` edit, not a separate spec.
- Toolchain floor: both distros clear the workspace floor
  (`rust-version = "1.88"`), so no rustup is needed in the build
  containers — Fedora 43 resolves rust 1.98.1, Tumbleweed ships rust
  1.98.1 (both re-measured 2026-10-08 with the same queries;
  toolchains move, so re-check at build time).
- `cargo-deny` exists on Fedora but has no openSUSE package, so no spec
  runs it: `THIRD-PARTY-LICENSES` arrives inside the vendored tarball
  (staged by `scripts/vendor-release.sh`) and the specs only install it.
  CI stages the tarball with `vendor-release.sh` itself, which does need
  `cargo-deny` — on openSUSE via `cargo install cargo-deny --locked`.
- The bar's `--check` on the shipped example needs a font to pass:
  `dejavu-sans-fonts` on Fedora, `dejavu-fonts` on openSUSE (each
  provides a path the bar's fallback list already probes:
  `crates/scootbar/src/font.rs`). The launcher's `--check` needs none:
  with no config file it validates the built-in defaults.

## Installed files

- `scoot`: `/usr/bin/scoot`, `/usr/bin/scoot-session` (the in-tree
  greeter launcher), `/usr/share/wayland-sessions/scoot.desktop`
  (`DesktopNames=scoot` is the greeter contract: the NixOS module ships
  the same key, and so do Fedora's own `gnome.desktop` session files —
  all of which trip the same `desktop-file-validate` notice, which CI
  accepts as the one known exception and which is why the spec does not
  run the validator at build time),
  the session user units (`%{_userunitdir}/scoot.service` with
  `ExecStart=/usr/bin/scoot --tty`, `scoot-session.target`,
  `scoot-shutdown.target` — the same files `resources/systemd/user/`
  carries and the NixOS module installs), the portal selection
  (`/usr/share/xdg-desktop-portal/scoot-portals.conf`),
  licenses (`%license`: `LICENSE`, `NOTICE`, `THIRD-PARTY-LICENSES`), and
  the output of `scoot --print-default-config` as `%doc`
  (`config.toml.example`, which documents the `[wallpaper]` handoff, so
  `scootbg` ships no separate example). Nothing is written to a user's
  home; uninstall leaves nothing behind.
- `scootbg`: `/usr/bin/scootbg` plus its licenses.
- `scootbar`: `/usr/bin/scootbar`, the `scootbar.service` user unit
  (`Restart=on-failure`, the same policy as
  `nix/modules/scootbar-home.nix`, bound to the shared
  `graphical-session.target` for standalone use, with the
  `%systemd_user_post`/`%systemd_user_preun` scriptlets), licenses, and
  a starting `bar.toml.example` (proven valid: `scootbar daemon --check`
  prints `ok`, in `%check` and again after install).
- `scootlaunch`: `/usr/bin/scootlaunch` plus its licenses (in `%check`
  and again after install: `--version`, and `--check` with no config
  file, which validates the built-in defaults).
- `%check` in every spec runs only headless-safe probes (`--version`,
  `--print-default-config`, `daemon --check`): the headless smoke test
  needs a runtime, so it runs after install in CI, not in the build.
- Binaries are stripped in `%install` (the workspace already builds
  `strip = true`): with `debug_package %{nil}` no find-debuginfo run
  strips them, and an unstripped build would ship its symtab.
- Accepted `rpmlint` warnings (errors are zero): `no-manual-page-for-binary`
  on all five entry points (the project ships no man pages yet, for any
  distro) and `no-documentation` on `scootbg` (its config surface is the
  `[wallpaper]` section documented in the `scoot` package's example, so it
  deliberately ships no `%doc` of its own) and on `scootlaunch` (its
  three optional keys are documented on the site, and a missing file is
  the built-in defaults, so it deliberately ships no `%doc` of its own
  either; `%check` covers `--check` on those defaults). One documented override file,
  `packaging/rpm/rpmlintrc`, used only by the openSUSE job: openSUSE's
  en_US dictionary flags the bare token `config` (config files and their
  `~/.config` paths), Fedora's does not — the filter is scoped to that
  exact token, and it is load-bearing, so a stale filter fails the lint
  instead of passing silently.
- `%doc` lands under `/usr/share/doc/` on Fedora and
  `/usr/share/doc/packages/` on openSUSE; the prove steps check the
  example at each distro's own path. Both container images skip docs by
  default (Fedora: `tsflags=nodocs`; openSUSE: a vendor
  `excludedocs.conf`), so both install steps override that — otherwise
  the `--check` on the installed example would test a file the package
  owns but the container never wrote.
- No `-debuginfo`/`-debugsource` subpackages (`debug_package %{nil}`):
  the vendored crate sources carry their upstream modes (executable
  `.rs` files plus a `#!` attribute line in one crate that is not a
  shebang), which fails `brp-mangle-shebangs` over `/usr/src/debug`.
  The revisit, if distro-grade inclusion ever wants debuginfo, is a
  mode-normalizing `find vendor -type f -exec chmod a-x {} +` in `%prep`
  (cargo checks content hashes, not modes, so the build is unaffected)
  — not yet proven, so not yet shipped.

## Architectures

`x86_64` and `aarch64`, through the same source as the release
binaries. CI builds `x86_64` only (GitHub-hosted runners); the specs
carry no `ExclusiveArch`, so the same SRPM builds on either.

## The COPR-style repository (designed, not created)

RPMs published through a hosted build
service, with Fedora's COPR as the host. What was checked,
and what stays manual:

- **Layout.** One COPR project (e.g. `scoot-sh/scoot`) holding one
  package per spec. Each package builds by the SCM pattern: the dist-git
  carries the spec plus a `.copr/Makefile` whose `srpm` target downloads
  the release's vendored tarball, renames it to the `Source0` basename,
  and calls `rpmbuild -bs`. No source is committed to the dist-git
  beyond the spec and the three support files beside it.
- **Chroots.** At setup time, enable in the COPR UI what the ticket
  needs and confirm each still exists: the current and prior Fedora
  releases plus Rawhide, on `x86_64` and `aarch64`, and whatever
  openSUSE chroots COPR offers then. Chroot availability moves (releases
  go EOL, new ones appear), so the enabled set is recorded in the
  project description, not here.
- **Not done here, on purpose.** No COPR account was touched, no project
  created, nothing uploaded, no token in any file: creating the project
  and the first build is a manual maintainer step (the COPR new-project
  flow plus `copr-cli buildscm`), exactly like the AUR push stays manual
  for the Arch packages. The `Source0` name is the release asset's real
  name, `scoot-<version>-vendored.tar.gz` (release.yml passes the version
  to `scripts/vendor-release.sh --label`; see the comment at the top of
  `scoot.spec`); the `.copr/Makefile` is written at setup, against that
  name.

## Cutting a release (maintainer)

`release-plz.yml` already bumped `Version` on the release PR
(`scripts/sync-packaging-versions`, committed right after release-plz
opens or updates the PR); the steps below are the manual fallback when
that sync is missing or wrong.

1. Nothing to rename: every release publishes
   `scoot-<version>-vendored.tar.gz`, the name each spec's `Source0`
   already downloads.
2. In the release PR, before it merges, bump `Version` in all four
   specs to the proposed version, reset `Release` to `1` and add a
   `%changelog` entry (run `rpm.yml` by hand from the release PR; it
   fails if `Version` is not the workspace version).
3. Write the `.copr/Makefile` per package against the real asset names,
   create the COPR project, enable the chroots above, and run the first
   `buildscm` by hand.
4. `rpmlint` the published RPMs once from the repository (the CI lint
   below runs on the build-tree RPMs; the repo pass confirms the signed
   artifacts are the same files).

## What CI proves

`.github/workflows/rpm.yml` stages the vendored tarball from HEAD with
`scripts/vendor-release.sh` in a Fedora 43 and a Tumbleweed container,
builds all four specs with `rpmbuild -ba` in each, lints every built RPM
with `rpmlint`, installs the set, and runs `scoot --version`,
`scootbg --version`, `scootbar --version`, `scootlaunch --version`,
`scootbar daemon --check` on
the shipped example, `scootlaunch --check` on the built-in defaults,
and — in the Fedora container only — the headless smoke test
(`scripts/smoke-test.sh`) against the installed binaries. The
`desktop-file-validate` gate on the session entry also runs in the
Fedora container only: the file is identical in both jobs, and
`rpm-build` itself requires `gawk`, which conflicts with the Tumbleweed
image's `busybox-gawk` (removed first in CI; `desktop-file-utils` stays
out for the same reason). The one thing CI cannot prove yet: the `Source0` URLs, which
404 until the first release is cut; until then the staged tarball is
placed under that same name, and only the URL part is
substitution.
