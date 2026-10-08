# RPM packages (Fedora and openSUSE)

Three spec files, one per shipped binary, built from the vendored-sources
tarball (`scripts/vendor-release.sh`: the tagged tree plus `vendor/`,
`.cargo/config.toml` and `THIRD-PARTY-LICENSES`, so the build is offline).
Nothing here is published anywhere: there is no COPR project, no repo, no
tag, no release in CI or out of it. The COPR layout below is designed and
documented only (the ticket's second half is a host account away).

| Spec | Package | Source tag | Version follows |
|---|---|---|---|
| `scoot.spec` | `scoot` | `scoot-v$Version` | `crates/scoot/Cargo.toml` (trio lockstep) |
| `scootbg.spec` | `scootbg` | `scoot-v$Version` | `crates/scootbg/Cargo.toml` (same trio tag) |
| `scootbar.spec` | `scootbar` | `scootbar-v$Version` | `crates/scootbar/Cargo.toml` (independent) |

There is no `scootctl` package: it is the client library inside the
`scoot` binary (`scoot msg`), versioned with the trio
(docs/versioning.md). `scoot` requires exactly `scootbg=$Version-$Release`:
the coupled pair installs at one version, and the bar requires no
compositor package at all (standard Wayland protocols only).

Why a hand-written spec and not `cargo-generate-rpm`: the ticket allows
either, and the generated file would need the same hand edits anyway
(the session entry, the bar's user unit, the license set including the
deny-generated inventory, the vendored offline build, the versioned
`scootbg` requirement). The spec is the whole package definition; what
`cargo-generate-rpm` would contribute (the file list, the soname
requirements) is three binaries RPM derives on its own.

## Dependencies, from data

`BuildRequires` is the `ldd`-derived list from the site's packaging page
(`site/src/content/docs/reference/packaging.md`), not memory: the
compositor links `libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`, and the wallpaper daemon and the
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
| `libgbm.so.1` / `libdrm.so.2` (`gpu-scanout` only) | `mesa-libgbm` / `libdrm` | same sonames, `optdepends`-equivalent: not required (see below) |

Notes:

- The `gpu-scanout` extras stay out of the shipped build (default feature
  set, same as the Arch packages): rebuilding with `--features
  gpu-scanout` is a local `rpmbuild` edit (the feature on the `cargo
  build` line, the two `-devel` packages beside it), not a separate spec.
- Toolchain floor: Fedora 43 ships rust 1.90, Tumbleweed rust 1.98
  (checked with the same queries); both clear the workspace floor
  (`rust-version = "1.87"`). No rustup needed in the build containers.
- `cargo-deny` exists on Fedora but has no openSUSE package, so no spec
  runs it: `THIRD-PARTY-LICENSES` arrives inside the vendored tarball
  (staged by `scripts/vendor-release.sh`) and the specs only install it.
  CI stages the tarball with `vendor-release.sh` itself, which does need
  `cargo-deny` — on openSUSE via `cargo install cargo-deny --locked`.
- The bar's `--check` on the shipped example needs a font present only
  to prove text shaping end to end: `dejavu-sans-fonts` on Fedora,
  `dejavu-fonts` on openSUSE (both provide a path the bar's fallback
  list already probes: `crates/scootbar/src/font.rs`).

## Installed files

- `scoot`: `/usr/bin/scoot`, `/usr/bin/scoot-session` (the in-tree
  greeter launcher), `/usr/share/wayland-sessions/scoot.desktop`
  (`DesktopNames=scoot` is the greeter contract: the NixOS module ships
  the same key, and so do Fedora's own `gnome.desktop` session files —
  all of which trip the same `desktop-file-validate` notice, which CI
  accepts as the one known exception and which is why the spec does not
  run the validator at build time),
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
- `%check` in every spec runs only headless-safe probes (`--version`,
  `--print-default-config`, `daemon --check`): the headless smoke test
  needs a runtime, so it runs after install in CI, not in the build.
- Binaries are stripped in `%install` (the workspace already builds
  `strip = true`): with `debug_package %{nil}` no find-debuginfo run
  strips them, and an unstripped build would ship its symtab.
- Accepted `rpmlint` warnings (errors are zero): `no-manual-page-for-binary`
  on all four entry points (the project ships no man pages yet, for any
  distro) and `no-documentation` on `scootbg` (its config surface is the
  `[wallpaper]` section documented in the `scoot` package's example, so it
  deliberately ships no `%doc` of its own). One documented override file,
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

Level 1 asks for repo-hosted RPMs published through a hosted build
service, with Fedora's COPR as the obvious host — and explicitly not to
assume its terms, chroots or architecture coverage. What was checked,
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
  for the Arch packages. The `Source0` basenames in the specs are
  provisional until the first release names the asset (see the comment
  at the top of `scoot.spec`); the `.copr/Makefile` is written then,
  against the real asset names, not now against guesses.

## Cutting a release (maintainer)

1. Name the release assets (the vendored tarballs the release job
   publishes) and set each spec's `Source0` basename to match (today a
   `scoot-<rev>-vendored` name the release job chooses; only that line
   changes per spec).
2. Bump `Version` in the moving specs to the released version (the trio
   together, the bar on its own) and reset `Release` to `1`.
3. Write the `.copr/Makefile` per package against the real asset names,
   create the COPR project, enable the chroots above, and run the first
   `buildscm` by hand.
4. `rpmlint` the published RPMs once from the repository (the CI lint
   below runs on the build-tree RPMs; the repo pass confirms the signed
   artifacts are the same files).

## What CI proves

`.github/workflows/rpm.yml` stages the vendored tarball from HEAD with
`scripts/vendor-release.sh` in a Fedora 43 and a Tumbleweed container,
builds all three specs with `rpmbuild -ba` in each, lints every built RPM
with `rpmlint`, installs the trio, and runs `scoot --version`,
`scootbg --version`, `scootbar --version`, `scootbar daemon --check` on
the shipped example, and — in the Fedora container only, which is what
the ticket's "done when" names — the headless smoke test
(`scripts/smoke-test.sh`) against the installed binaries. The one thing
it cannot prove yet: the `Source0` URLs, which 404 until the first
release is cut; until then the staged tarball is placed under the
provisional basename, and only the URL part is substitution.
