# Arch Linux packages

Eight PKGBUILDs, one per shipped binary in each of two lines: a release
build from the tagged tarball, and a `-git` build tracking `main`. Nothing here is published anywhere: there is no AUR
push, no tag, no release in CI or out of it. AUR submission stays a
manual maintainer step (below).

| Directory | Package | Source | Version follows |
|---|---|---|---|
| `scoot/` | `scoot` | tag `v$pkgver` | the workspace version (`Cargo.toml`) |
| `scootbg/` | `scootbg` | tag `v$pkgver` | the same |
| `scootbar/` | `scootbar` | tag `v$pkgver` | the same |
| `scootlaunch/` | `scootlaunch` | tag `v$pkgver` | the same |
| `scoot-git/` | `scoot-git` | `main` | `pkgver()`: last `v*` tag, commits since it, short hash (`0.2.1.r14.g615608c`) |
| `scootbg-git/` | `scootbg-git` | `main` | the same scheme |
| `scootbar-git/` | `scootbar-git` | `main` | the same scheme |
| `scootlaunch-git/` | `scootlaunch-git` | `main` | the same scheme |

One tag releases every binary at the repository's one version
(docs/versioning.md), so all four release packages carry the same
`pkgver`, and the `-git` `pkgver` is the binaries' own build label
(`scoot --version` says `0.2.1+14.g615608c`). Before the first tag the
`-git` scheme counts every commit over the workspace version. There is
no `scootctl` package: it is the client library inside the `scoot`
binary (`scoot msg`). `scoot` depends on exactly `scootbg=$pkgver`:
the coupled pair installs at one version, and the bar and the launcher
depend on no compositor package at all (standard Wayland protocols
only).

## Dependencies, from data

`depends` is the `ldd`-derived list from the site's packaging page
(`site/src/content/docs/reference/packaging.md`): the
compositor links `libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0`, the launcher links
`libxkbcommon.so.0` for its keyboard (plus `libm.so.6` through the SVG
icon stack, which rides inside `glibc` everywhere, so no extra
dependency names it), and the wallpaper daemon and
the bar link nothing past the C library. The Arch package names:

- `libseat.so.1` comes from **`seatd`**, not `libseat`: Arch renamed
  the package (it `Provides: libseat.so`, `Replaces: libseat`), on
  x86_64 and on Arch Linux ARM alike. `pacman -S libseat` no longer
  resolves, which is also why the site's releases page names `seatd`
  on its Arch line.
- `libudev.so.1` comes from `systemd-libs`; `libgbm.so.1`/`libdrm.so.2`
  (the GPU scanout tier's run-time libraries, auto-detected -- the shipped
  build links neither) from `mesa`/`libdrm`, carried
  as `optdepends` since a GPU-less box never needs them.
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
unneeded because it is a spawned program dependency,
not a linked library, which a linkage linter cannot see.

## Installed files

- `scoot`: `/usr/bin/scoot`, `/usr/bin/scoot-session` (the in-tree
  greeter launcher; degrades to `exec scoot --tty` with no systemd
  user manager), `/usr/share/wayland-sessions/scoot.desktop`,
  the session user units (`/usr/lib/systemd/user/scoot.service` with
  `ExecStart=/usr/bin/scoot --tty`, `scoot-session.target`,
  `scoot-shutdown.target` — the same files `resources/systemd/user/`
  carries and the NixOS module installs), the portal selection
  (`/usr/share/xdg-desktop-portal/scoot-portals.conf`),
  licenses under `/usr/share/licenses/scoot/`, and the output of
  `scoot --print-default-config` as
  `/usr/share/doc/scoot/config.toml.example` (it documents the
  `[wallpaper]` handoff, so `scootbg` ships no separate example).
  The session entry names the session `scoot` via `DesktopNames`
  (XDG_CURRENT_DESKTOP), the same key sway and the Nix-built entry
  carry. Nothing is written to a user's home; uninstall leaves
  nothing behind.
- `scootbg`: `/usr/bin/scootbg` plus its licenses.
- `scootbar`: `/usr/bin/scootbar`, the `scootbar.service` user unit
  (`Restart=on-failure`, the same policy as
  `nix/modules/scootbar-home.nix`, bound to the shared
  `graphical-session.target` for standalone use), licenses, and a
  starting `bar.toml.example` (proven valid: `scootbar daemon --check`
  prints `ok`).
- `scootlaunch`: `/usr/bin/scootlaunch` plus its licenses (proven
  starting: `scootlaunch --check` with no config file exits 0 on the
  built-in defaults).

`options=('!debug' '!lto')`
everywhere: `!debug` because the auto-split `-debug` subpackages ship a
build-id symlink into the main package that a per-package lint reads as
dangling, and carry no value for AUR users; `!lto` because makepkg's
default `CFLAGS` carry `-flto=auto`, which bakes GCC LTO bytecode into
the compositor's static libgbm stub (`crates/scoot/build.rs`) that
`ld.lld` plus GCC's `liblto_plugin` then drops as undefined `gbm_*`
(the plugin claims bytecode files and mishandles them under lld, while
bfd consumes them fine -- which is also why the Fedora RPM builds never
saw it). Rust's own `lto = "fat"` is untouched: only the C compile
drops LTO, exactly what `!lto` scopes. The `-git` packages install their
licenses under their own name (`/usr/share/licenses/scoot-git/`, where
namcap requires them) while their docs follow the provided name
(`/usr/share/doc/scoot/`).

## Architectures

`arch=('x86_64' 'aarch64')`: Arch Linux ARM carries the same package
names for every dependency above (`seatd` verified against its package index), so the ARM
build is the same compile. CI builds `x86_64` only (the `archlinux`
container image ships no ARM variant).

## Cutting a release (maintainer)

`release-plz.yml` already bumped `pkgver` on the release PR
(`scripts/sync-packaging-versions`, committed right after release-plz
opens or updates the PR); the steps below are the manual fallback when
that sync is missing or wrong.

1. In the release PR, before it merges, bump `pkgver` in all four
   release packages to the proposed version and regenerate `.SRCINFO`
   (run `arch.yml` by hand from the release PR; it fails if `pkgver` is
   not the workspace version).
2. Once the tag exists, `updpkgsums` in each directory (the `SKIP`
   checksums are placeholders until there is a tag tarball to hash).
3. `makepkg --printsrcinfo > .SRCINFO` in each touched directory and
   commit it (CI fails when the committed file drifts).
4. Copy each directory's contents into its AUR repo (`scoot`,
   `scootbg`, `scootbar`, `scootlaunch`, `scoot-git`, ...) and push there by hand.
   Build `scootbg` before `scoot`: scoot's versioned dependency on it
   resolves against no repository, so it installs from the just-built
   package (CI does the same; an AUR helper builds the chain in this
   order on its own).

XWayland is compiled in by default (the `xwayland` cargo feature is on
in the default set); enabling it at run time needs the `Xwayland` binary
on `PATH` plus scoot's runtime opt-in (`--xwayland` / `[xwayland]
enabled`). A local `makepkg` edit is only needed to build *without* it
(`--no-default-features`, the cargo-only minimal build), not a separate
package. The GPU scanout tier needs no such edit: it
is on by default and detects `mesa`/`libdrm` at run time.

## What CI proves

`.github/workflows/arch.yml` builds all eight PKGBUILDs in an
`archlinux:base` container with `makepkg`, lints each PKGBUILD and
each built package with `namcap`, installs the release set and the
`-git` set in turn, and runs `scoot --version`, `scootbar daemon
--check` on the shipped example, `scootlaunch --check` on the built-in
defaults, and the headless smoke test
(`scripts/smoke-test.sh`) against the installed binaries. The one
thing it cannot prove yet: the release tarball URLs, which 404 until
the first tag is cut; until then the release PKGBUILDs build against
a same-layout tarball of HEAD (same top directory, same lockfile),
and only the URL-plus-checksum part is substitution.
