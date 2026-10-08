# Arch Linux packages

Six PKGBUILDs, one per shipped binary in each of the two lines the
ticket ([arch-package](../../docs/backlog/packaging/arch-package.md))
asks for: a release build from the tagged tarball, and a `-git` build
tracking `main`. Nothing here is published anywhere: there is no AUR
push, no tag, no release in CI or out of it. AUR submission stays a
manual maintainer step (below).

| Directory | Package | Source | Version follows |
|---|---|---|---|
| `scoot/` | `scoot` | tag `scoot-v$pkgver` | `crates/scoot/Cargo.toml` (trio lockstep) |
| `scootbg/` | `scootbg` | tag `scoot-v$pkgver` | `crates/scootbg/Cargo.toml` (same trio tag) |
| `scootbar/` | `scootbar` | tag `scootbar-v$pkgver` | `crates/scootbar/Cargo.toml` (independent) |
| `scoot-git/` | `scoot-git` | `main` | `pkgver()`: crate version, commit count, short hash |
| `scootbg-git/` | `scootbg-git` | `main` | same scheme over `crates/scootbg/Cargo.toml` |
| `scootbar-git/` | `scootbar-git` | `main` | same scheme over `crates/scootbar/Cargo.toml` |

There is no `scootctl` package: it is the client library inside the
`scoot` binary (`scoot msg`), versioned with the trio
(docs/versioning.md). `scoot` depends on exactly `scootbg=$pkgver`:
the coupled pair installs at one version, and the bar depends on no
compositor package at all (standard Wayland protocols only).

## Dependencies, from data

`depends` is the `ldd`-derived list from the site's packaging page
(`site/src/content/docs/reference/packaging.md`), not memory: the
compositor links `libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`, and the wallpaper daemon and
the bar link nothing past the C library (measured on an aarch64
release build: `scoot` 6.5 MB with those five sonames, `scootbg`
1.6 MB and `scootbar` 2.4 MB libc-only). The Arch package names:

- `libseat.so.1` comes from **`seatd`**, not `libseat`: Arch renamed
  the package (it `Provides: libseat.so`, `Replaces: libseat`), on
  x86_64 and on Arch Linux ARM alike. `pacman -S libseat` no longer
  resolves, which is also why the site's releases page names `seatd`
  on its Arch line.
- `libudev.so.1` comes from `systemd-libs`; `libgbm.so.1`/`libdrm.so.2`
  (the `gpu-scanout` feature's extras) from `mesa`/`libdrm`, carried
  as `optdepends` since the shipped build is the default feature set.
- `sh` is a dependency of the `scoot` packages (the `scoot-session`
  launcher is a `sh` script, and `sh` is what the distro's own
  packages depend on — `bash` provides it); `libgcc` is one everywhere
  (every binary links `libgcc_s`, and namcap names it the provider —
  the `gcc-libs` entry it replaced covered nothing the binaries use).
  `glibc` stays: the binaries link it and namcap wants it named.
- `cargo-deny` generates each package's `THIRD-PARTY-LICENSES` at
  build time (lockfile-only `cargo deny list`, no network); it is in
  `extra` on both architectures.

`namcap` is clean on errors; the remaining warnings are each
explained, not waived: `scootbg` on the `scoot` packages reads as
unneeded because it is a spawned program dependency (the lockstep
pair), not a linked library, which a linkage linter cannot see.

## Installed files

- `scoot`: `/usr/bin/scoot`, `/usr/bin/scoot-session` (the in-tree
  greeter launcher; degrades to `exec scoot --tty` with no systemd
  user manager), `/usr/share/wayland-sessions/scoot.desktop`,
  licenses under `/usr/share/licenses/scoot/`, and the output of
  `scoot --print-default-config` as
  `/usr/share/doc/scoot/config.toml.example` (it documents the
  `[wallpaper]` handoff, so `scootbg` ships no separate example).
  Nothing is written to a user's home; uninstall leaves nothing behind.
- `scootbg`: `/usr/bin/scootbg` plus its licenses.
- `scootbar`: `/usr/bin/scootbar`, the `scootbar.service` user unit
  (`Restart=on-failure`, the same policy as
  `nix/modules/scootbar-home.nix`, bound to the shared
  `graphical-session.target` for standalone use), licenses, and a
  starting `bar.toml.example` (proven valid: `scootbar daemon --check`
  prints `ok`).

Two deliberate packaging choices, both namcap-driven: `options=('!debug')`
everywhere (the auto-split `-debug` subpackages ship a build-id symlink
into the main package that a per-package lint reads as dangling, and
carry no value for AUR users), and the `-git` packages install their
licenses under their own name (`/usr/share/licenses/scoot-git/`, where
namcap requires them) while their docs follow the provided name
(`/usr/share/doc/scoot/`, which the install notes reference).

## Architectures

`arch=('x86_64' 'aarch64')`: Arch Linux ARM carries the same package
names for every dependency above (`seatd`, `cargo-deny` verified
against its own package index), and the tree already ships
`aarch64-linux` release binaries from the same source, so the ARM
build is the same compile. CI builds `x86_64` only (the `archlinux`
container image ships no ARM variant); the `aarch64` entry is one
line to drop if it ever misbehaves.

## Cutting a release (maintainer)

1. Bump `pkgver` in the moving packages to the released version (the
   trio together, the bar on its own).
2. `updpkgsums` in each touched directory (the `SKIP` checksums are
   placeholders: no tag exists yet to hash).
3. `makepkg --printsrcinfo > .SRCINFO` in each touched directory and
   commit it (CI fails when the committed file drifts).
4. Copy each directory's contents into its AUR repo (`scoot`,
   `scootbg`, `scootbar`, `scoot-git`, ...) and push there by hand.
   Build `scootbg` before `scoot`: scoot's versioned dependency on it
   resolves against no repository, so it installs from the just-built
   package (CI does the same; an AUR helper builds the chain in this
   order on its own).

Rebuilding with the opt-in tiers (`--features gpu-scanout` for the
GBM scanout path, `--features xwayland` plus the `xwayland` runtime
package) is a local `makepkg` edit, not a separate package: add the
feature to the `cargo build` line and the matching `optdepends` to
`depends`.

## What CI proves

`.github/workflows/arch.yml` builds all six PKGBUILDs in an
`archlinux:base` container with `makepkg`, lints each PKGBUILD and
each built package with `namcap`, installs the release trio and the
`-git` trio in turn, and runs `scoot --version`, `scootbar daemon
--check` on the shipped example, and the headless smoke test
(`scripts/smoke-test.sh`) against the installed binaries. The one
thing it cannot prove yet: the release tarball URLs, which 404 until
the first tag is cut; until then the release PKGBUILDs build against
a same-layout tarball of HEAD (same top directory, same lockfile),
and only the URL-plus-checksum part is substitution.
