# Debian and Ubuntu packages

Three `.deb`s, one per shipped binary, built with cargo from the
vendored sources and staged with `dpkg-deb` (ticket:
[deb-package](../../docs/backlog/packaging/deb-package.md)). Nothing
here is published anywhere: no repository upload, no tag, no release
in CI or out of it. The signed apt repository is designed below and
its no-secret pieces ship here; the key itself is the maintainer's
step.

| Package | Source | Version follows |
|---|---|---|
| `scoot` | trio tag `scoot-v$ver` | `crates/scoot/Cargo.toml` (trio lockstep) |
| `scootbg` | trio tag `scoot-v$ver` | `crates/scootbg/Cargo.toml` (same trio tag) |
| `scootbar` | bar tag `scootbar-v$ver` | `crates/scootbar/Cargo.toml` (independent) |

There is no `scootctl` package: it is the client library inside the
`scoot` binary (`scoot msg`), versioned with the trio
(docs/versioning.md). `scoot` depends on exactly `scootbg (= $ver)`:
the coupled pair installs at one version, and the bar depends on no
compositor package at all (standard Wayland protocols only).

## Why `dpkg-deb`, not `cargo-deb`

The ticket's level 1 names `cargo-deb` with metadata in each crate's
`Cargo.toml`. This tree builds the `.deb`s with `cargo` plus
`dpkg-deb` instead (a maintainer-away decision, trivially reversible:
`build-deb.sh` is one script and the manifests are untouched):

- It mirrors the Arch shape (`packaging/arch` builds with `cargo` and
  stages files; no manifest metadata), so the two distro paths stay
  one idea.
- `cargo-deb` would be a new build dependency that itself must vendor
  offline, and its metadata in `Cargo.toml` would entangle the
  versioning policy (`scripts/version check` reads every manifest).
- The ticket's `$auto` requirement is kept literally: `Depends`
  versions come out of `dpkg-shlibdeps -O` at build time, checked
  against the `ldd`-derived allow-list (below) -- a renamed or new
  system library fails the build instead of shipping a wrong
  dependency.

A full `debian/` source package (the ticket's level 2, for archive
inclusion) is out of scope until there is demand, as the ticket says.

## Dependencies, from data

`Depends` is derived at build time by `dpkg-shlibdeps`, and every
package name it emits must already be in the allow-list in `deb-meta.py`
(`ALLOW`), which is the `ldd`-derived table from the site's packaging
page (`site/src/content/docs/reference/packaging.md`) -- not memory.
The compositor links `libinput.so.10`, `libseat.so.1`, `libudev.so.1`,
`libpixman-1.so.0`, `libxkbcommon.so.0` (plus the C library and its
closure); the wallpaper daemon and the bar link nothing past the C
library (measured on a release build: `scootbg` and `scootbar` are
libc-only). On Debian 12 / Ubuntu 24.04 and newer the sonames resolve
to `libinput10`, `libseat1`, `libudev1`, `libpixman-1-0`,
`libxkbcommon0` (plus `libc6`, `libgcc-s1` via shlibdeps). The
`gpu-scanout` extras (`libgbm1`, `libdrm2`) are not in any shipped
`.deb`: the shipped build is the default feature set, and rebuilding
with the opt-in tiers is a local `build-deb.sh` run, not a separate
package.

The bar `Suggests: fonts-dejavu-core`: it draws text only with a font,
and the shipped example points at DejaVu (the first path the bar
probes, `crates/scootbar/src/font.rs`).

## Toolchain: the distro's rustc is too old, so the container pins one

The workspace floor is `rust-version = "1.87"`. Bookworm ships 1.63,
noble 1.75, trixie 1.85.1 -- none builds the tree (the site packaging
page's table, checked 2026-10-07). Per the ticket this picks the
first option: the build container installs a newer toolchain, which
is fine for a repo-hosted package. It is the same rustup pin the
release builds use (`RUST_TOOLCHAIN: 1.97.0` in `release.yml` and
`deb.yml`), and the glibc floor is the container's: bookworm's 2.36,
asserted in CI, so one build runs on trixie, noble and newer.

## Installed files

- `scoot`: `/usr/bin/scoot`, `/usr/bin/scoot-session` (the in-tree
  greeter launcher; degrades to `exec scoot --tty` with no systemd
  user manager), `/usr/share/wayland-sessions/scoot.desktop`,
  copyright, the deny-generated `THIRD-PARTY-LICENSES`, a
  `changelog.Debian.gz` (required: our versions carry a Debian
  revision, so the packages are non-native and lintian fails without
  one; its entry points at the per-package release notes, where this
  tree's history lives) and a `README.Debian` under
  `/usr/share/doc/scoot/`, and the output of
  `scoot --print-default-config` as
  `/usr/share/doc/scoot/config.toml.example` (it documents the
  `[wallpaper]` handoff, so `scootbg` ships no separate example).
  Nothing is written to a user's home; with no conffiles and no
  maintainer scripts, removal leaves nothing behind.
- `scootbg`: `/usr/bin/scootbg` plus its licenses and changelog.
- `scootbar`: `/usr/bin/scootbar`, the `scootbar.service` user unit
  (`Restart=on-failure`, the same policy as
  `nix/modules/scootbar-home.nix`, bound to the shared
  `graphical-session.target` for standalone use), licenses, a
  changelog, a `README.Debian`, and a starting `bar.toml.example`
  (proven valid: `scootbar daemon --check` prints `ok`).

Lintian posture: no errors; the one standing warning is
`no-manual-page` on each binary (explained, not overridden -- the
binaries' `--help` plus the site reference are the documentation;
manpages are a later addition, not this ticket). `Standards-Version`
is deliberately absent from the binary control: it belongs in source
packaging, and bookworm's lintian flags it as `unknown-field` there.

`Maintainer` names the upstream copyright holder's public commit
identity (`deb-meta.py`); the signing key below is separate from it.

## Architectures

`amd64` and `arm64`, each built natively (the same runner pair the
releases use: `ubuntu-latest` and `ubuntu-24.04-arm`, no emulation,
no cross). Local proof runs on whichever is at hand; CI proves both.

## Building (maintainer)

From a clean checkout (before the first release) or from the published
vendored tarball after it -- the same layout either way (`source/`
with `vendor/`, `.cargo/config.toml`, `THIRD-PARTY-LICENSES`):

```sh
scripts/vendor-release.sh --out ./dist          # before the first release: stage HEAD
# after it: tar -xzf scoot-<rev>-vendored.tar.gz -C ./dist
packaging/deb/build-deb.sh --source ./dist/source --out ./debs
```

`--revision N` sets the Debian revision (`0.1.0-1` the first time; a
rebuild of the same upstream version bumps it). `scoot`'s `scootbg`
pin always names the `scootbg` version the same run builds, so the
pair can never skew.

## The signed apt repository (designed, not yet live)

Decision: `reprepro` (maintainer-away, reversible -- the repo is
static files either way). It is in the Debian archive (no third-party
binary to trust), handles several distributions in one repository
(`conf/distributions`), and signs with an ordinary GPG key at export
time. `aptly` would do the same job with its own database and more
moving parts; if it ever fits better, only the publisher side
changes -- the `.deb`s and the client setup below do not.

What ships here (needs no secret): `make-repo-index.sh` builds the
unsigned metadata (`Packages`, `Packages.gz`, `Release`) from a
directory of `.deb`s. CI runs it and installs from the resulting
tree, which proves the metadata shape; the install uses
`[trusted=yes]` there and only there, because CI has no key.

What the maintainer must provide before the repository goes live
(nothing here invents any of it):

1. A GPG key reserved for the repository (generated offline, ideally
   signing-only), and its key id and fingerprint, recorded in
   `conf/distributions` (`SignWith:`) when `reprepro` is set up.
2. The public half, published where clients can fetch it over HTTPS
   (the repository host, a keyserver, or a `scoot-archive-keyring`
   package -- whichever, the client setup names exactly one).
3. The repository host and base URL (static file hosting is enough:
   `dists/`, `pool/`, the key).
4. The distributions to publish (`bookworm`, `trixie`, `noble`, ... --
   one `reprepro` codename each, all served from the bookworm-built
   `.deb`s while the glibc floor stays 2.36).
5. Where the private half lives for CI publishing (a hardware token, a
   CI secret, or a maintainer-side sign-and-upload -- decided then;
   no secret name or value is pre-created here).

Publishing then looks like this (maintainer runs it, not CI, until
item 5 is decided):

```sh
reprepro -b /srv/apt/scoot includedeb bookworm scoot_0.1.0-1_amd64.deb
reprepro -b /srv/apt/scoot includedeb bookworm scoot_0.1.0-1_arm64.deb
# ... every .deb, every codename; reprepro signs Release into InRelease
# with the item-1 key at export time.
```

and clients point at it with the key pinned (no key ever lands in the
deprecated global keyring):

```sh
# /etc/apt/sources.list.d/scoot.sources (DEB822):
# Types: deb
# URIs: https://apt.scoot.sh/
# Suites: bookworm
# Components: main
# Signed-By: /usr/share/keyrings/scoot-archive-keyring.gpg
```

(Host and key paths are the item-2/item-3 decisions; this file is the
shape, not the values.) Never tag, release or push to any registry
from here: attaching the `.deb`s to the GitHub Release and feeding
the repository stays maintainer-run until the key exists.

## What CI proves

`.github/workflows/deb.yml` stages the vendored tree
(`scripts/vendor-release.sh`, the same entry point the release job
uses), builds all three `.deb`s `--release --locked --offline` in a
`debian:bookworm-slim` container with the pinned toolchain on each
architecture, derives and allow-list-checks `Depends`, lints with
`lintian`, builds the unsigned repository index, and installs from it
in plain `bookworm-slim` (and Ubuntu 24.04) containers: every
`--version`, the bar's `--check` on its shipped example, and the
headless smoke test (`scripts/smoke-test.sh`) against the installed
binaries. What it cannot prove yet: the signature itself (there is no
key) and the release-tarball URLs (there is no source URL at all --
the script builds a source directory, so nothing 404s).
