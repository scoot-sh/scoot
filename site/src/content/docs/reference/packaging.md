---
title: Package scoot offline
description: "Build scoot from vendored sources with no network: toolchain, system libraries per feature, forks, licenses."
---

Build scoot the way a distro builder does: from a source tree that
needs no network, with a toolchain and system libraries you provide.
Start here if you maintain the Arch, deb or rpm package. The Arch
PKGBUILDs (release and `-git`) build on this page
([`packaging/arch/`][arch-pkg]); the deb
(`packaging/deb`, `deb.yml`) and rpm recipes have landed too
([deb][deb-pkg], [rpm][rpm-pkg]). All three
workflows are manual-only and no packages are shipped yet.

[arch-pkg]: https://github.com/scoot-sh/scoot/blob/main/packaging/arch/README.md
[deb-pkg]: https://github.com/scoot-sh/scoot/blob/main/docs/backlog/resolved/deb-package-done.md
[rpm-pkg]: https://github.com/scoot-sh/scoot/blob/main/docs/backlog/resolved/rpm-package-done.md

## Get the sources

Releases ship a vendored tarball: the exact tagged tree plus `vendor/`
(every dependency, crates.io and git alike), `.cargo/config.toml`
(source replacement pointing at `vendor/`), `THIRD-PARTY-LICENSES` (every
locked crate and its license) and `OFFLINE-BUILD.txt` (the same build
below, readable with no network). The same tarball is one command from any clean checkout:

```sh
scripts/vendor-release.sh --out ./dist --check-offline
```

`--check-offline` builds the staged tree under an empty `CARGO_HOME`
with `CARGO_NET_OFFLINE` before tarring it, so any network touch fails
the run instead of reaching you. The tree it stages is `git archive
HEAD` — reproducible from the commit, never the working tree.

Unpack and build (from the unpacked `source/` directory):

```sh
cargo build --workspace --locked --offline
```

`--locked` is not optional: `Cargo.lock` is committed, and every build
here and in CI uses it, so the tree you audit is the tree that builds.
The two git dependencies — the Smithay fork and the `wayland-backend`
fork — resolve out of `vendor/` through this snippet (already in the
tarball's `.cargo/config.toml`; the part most likely to break, and the
part CI's vendor job re-proves on every dependency change):

```toml
[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/scoot-sh/smithay?rev=fdf424d633fc15736a4a9c967b4e04a9e1e90077"]
git = "https://github.com/scoot-sh/smithay"
rev = "fdf424d633fc15736a4a9c967b4e04a9e1e90077"
replace-with = "vendored-sources"

[source."git+https://github.com/scoot-sh/wayland-rs?rev=70f81e005179463fb4d46ac68045a7f21eb5aff2"]
git = "https://github.com/scoot-sh/wayland-rs"
rev = "70f81e005179463fb4d46ac68045a7f21eb5aff2"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
```

## What you need

A Rust toolchain at or past the workspace floor, and the system
libraries below. Versions are what `ldd` reported on a release build
(`cargo build --release -p scoot`, aarch64, 2026-10-07); sonames are
what your package depends on, store paths are NixOS's and not yours.

| Feature set | Links (sonames) | Release `scoot` |
|---|---|---|
| default (GPU scanout tier + `xwayland`, both on) | libinput.so.10, libseat.so.1, libudev.so.1, libpixman-1.so.0, libxkbcommon.so.0 (plus glibc, libgcc_s and their transitive closure: libevdev, libmtdev, libwacom, libgudev, libsystemd, glib); no new system library for `xwayland` (pure-Rust X protocol over a socket), but needs the `Xwayland` binary on `PATH` at run time | 8.3 MB |
| `--no-default-features` (CPU-only minimal) | as default, minus nothing linked (there is neither GBM nor X11 code to link) | 6.7 MB |

Notes on the table: sizes are `cargo build --release -p scoot` on
aarch64 (previously 6.8 MB default without the tier and 7.4 MB with
`--features gpu-scanout`; the 8.3 MB default includes `xwayland`, measured as the
`+xwayland` delta before the fold). The single default build deliberately links
no GPU stack even though it compiles the scanout tier — it must run on a box with no GPU libraries
installed, and CI fails a default build that references `libgbm` (with a
positive control: the default binary must carry the stub's `libgbm.so.1`
`dlopen` literal, or the negative check is vacuous). `libEGL`/`libGLESv2`
never appear in `ldd` for any build:
the GLES renderer reaches them through `dlopen`. What the scanout tier
buys is the [GBM scanout tier](../scoot/backends.md#which-renderer-draws-the-frames):
without it (`--no-default-features`)
`--tty` presents through DRM dumb buffers. On Debian 13 the sonames
above resolve to `libinput10`, `libseat1`, `libudev1`,
`libpixman-1-0`, `libxkbcommon0` (plus `libgbm1`, `libdrm2` as
run-time `Suggests`, never `Depends`) — checked against trixie's own
`Contents-amd64.gz`;
other distros map the same sonames through their own file search.

`scootlaunch` links two system libraries past the C library:
`libxkbcommon.so.0` for its panel keyboard and `libm.so.6` (through
the default-on SVG icon stack; a release build's `NEEDED` is
`libxkbcommon.so.0`, `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`,
re-measured 2026-10-19 on aarch64 at 2167544 bytes stripped).
`libm.so.6` ships inside the C library package everywhere, so the
packaging delta is still one name: `libxkbcommon.so.0` resolves to
`libxkbcommon0` on Debian, Ubuntu and openSUSE and to `libxkbcommon`
on Fedora and Arch — the name each of the four package recipes depends
on. The headless commands (`--list`, `--query`, `--run-id`) never call
either library, but the loader still needs them present to start the
binary at all.

`scootnotify` links nothing past the C library, like the bar and the
wallpaper daemon: no system library beside it (the deb allow-list and
the rpm and Arch recipes carry no other name for it). Its cards draw
text only with a font, but without one the daemon stays headless and
the bus still serves — so the font rides as a suggestion (deb
`Suggests`, Arch `optdepends`; the rpm names none), never a dependency.
The same holds for the look swap itself: since the fallback chain
`apply` carries a missing family with a note instead of refusing, no
font package needs promoting to a dependency on any channel. Bookworm
ships no package for the Nerd families the shipped looks name
(`fonts-firacode` is the unpatched family), so the suggestion stays
DejaVu plus the fallback.

`scoot-globalshortcuts-portal` links nothing past the C library either:
no system library beside it, and it draws nothing, so no font even as
a suggestion.

### Toolchain floor: Rust 1.88

The workspace sets `rust-version = "1.88"`. Which distro releases ship
at least that (checked 2026-10-07 against each distro's own package
metadata — verify again at packaging time, toolchains move):

| Distro | rustc | Builds from the distro toolchain? |
|---|---|---|
| Debian 13 trixie | 1.85.1 | No — use rustup or trixie-backports/sid |
| Debian 12 bookworm | 1.63 | No |
| Debian sid | 1.96.1 | Yes |
| Ubuntu 24.04 noble | 1.75 | No |
| Ubuntu 25.10 questing | 1.85.1 | No |
| Ubuntu 26.04 resolute | 1.93.1 | Yes |
| Fedora 43 | 1.99.0 | Yes |
| Arch | 1.99.0 | Yes |
| Alpine edge | 1.99.0 | Yes |
| openSUSE Tumbleweed | 1.98.1 | Yes |
| Nix / NixOS | the flake's pinned toolchain | Yes (never the distro's) |

Debian stable and Ubuntu up to 25.10 cannot build scoot from their own
toolchain today: that is a rustup (or newer distro) requirement, not a
place to relax the floor.

## Forks: what is pinned and why

Two dependencies are scoot-sh forks, pinned by exact rev (the full
list, with the alternatives considered for each, is
[dev/forks.md](https://github.com/scoot-sh/scoot/blob/main/dev/forks.md)):

| Fork | Pinned rev | Why it exists |
|---|---|---|
| [scoot-sh/smithay](https://github.com/scoot-sh/smithay/tree/scoot/cursor-dmabuf-storage) (`scoot/cursor-dmabuf-storage`, on upstream `0ff00983`) | `fdf424d6` | 27 carried commits: a syncobj-timeline `Drop`, XWayland selection/drag fixes, a pixman upscale-edge clamp, a buffer-scale-without-new-buffer fix, an XSETTINGS flush, compositor-owned dma-buf scanout, and a clean shutdown on a lost seat |
| [scoot-sh/wayland-rs](https://github.com/scoot-sh/wayland-rs/tree/scoot/server-fd-queue-cap-adaptive) (`scoot/server-fd-queue-cap-adaptive`, on the 0.3.17 release) | `70f81e00` | bounds the backend's queue of received-but-unclaimed fds, so one idle client cannot fill the compositor's fd table; pinned as `[patch.crates-io]` |

A fork is the last resort here, and each entry records what was tried
instead. Nothing is sent upstream from this project; whether a fix is
offered upstream is the maintainer's later decision. Repinning either
fork changes its checkout hash: the Nix build records both hashes and
fails loudly on drift, and the vendor job re-proves the offline build.

## Licenses and advisories

scoot itself is MIT. Every third-party crate in the tarball is listed
with its license in `THIRD-PARTY-LICENSES`, generated by `cargo deny
list` at staging time. CI enforces the allow-list in `deny.toml`
(permissive only: MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0,
Zlib, Unlicense, 0BSD) and fails on anything outside it — copyleft
included, which needs an explicit maintainer decision recorded in that
file before it can land. The same tool checks RustSec advisories on
every dependency change and on a weekly schedule; two accepted risks
are recorded in `deny.toml` with reasons (an archived proc-macro with
no runtime exposure, and an unmaintained font parser that reads only
local fonts), and anything new is a visible red.

## Symptoms

- *The offline build tries the network.* `.cargo/config.toml` is
  missing or not this file: the build must run from the unpacked
  tarball (or a `vendor-release.sh` staging tree), not from a bare
  `git clone`. With the config in place and `--offline`, cargo fails
  rather than fetching — that failure names the crate it could not
  find vendored.
- *`error: the lock file needs updating`.* The build was run without
  `--locked`, or the tree was modified after staging. Re-stage from a
  clean checkout and keep `--locked`.
- *`rustc 1.XX is not supported`.* The toolchain predates the floor
  (see the table above): rustup to 1.88 or newer, or a distro release
  whose toolchain qualifies.
- *`Xwayland must be on PATH` at run time.* An `--features xwayland`
  build on a box without XWayland installed: the feature builds fine
  anywhere, but starting it needs the X server binary beside it.
