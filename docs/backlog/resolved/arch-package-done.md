---
title: "Arch Linux packages: PKGBUILDs (release and `-git`), AUR, built and linted in CI"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Arch Linux packages

Filed 2026-09-29. Serves **daily-drive**. Arch is the smallest packaging
target (a PKGBUILD is one file), so it goes first of the three.

## What to build

- **Split packages**, matching the independent versions
  ([independent-versioning](./independent-versioning-done.md)): `scoot`, `scootctl`,
  `scootbg`, `scootbar`, later `scootnotify`, `scootlaunch`. The coupled pair
  (`scoot` and `scootbg`) declares the compatible range.
- A **release** PKGBUILD per package (from the tagged tarball) and a `-git`
  variant tracking `main`. AUR first; official repositories are a later
  question.
- **Dependencies from data**: derive `depends` from `ldd` of a release build
  (and the extra ones the `gpu-scanout` feature links, as an optional
  dependency), not from memory.
- **Installed files**: the Wayland session entry
  (`/usr/share/wayland-sessions/scoot.desktop`), license files including the
  third-party inventory, and example config or `scoot --print-default-config`
  text under `/usr/share/doc`. A package never writes to a user's home or
  edits their config, and uninstall leaves nothing behind.
- **Bar restart**: a systemd user unit for `scootbar` (see
  [robustness-and-limits](../../scootbar/backlog/robustness-and-limits.md)).
- **CI**: build every package in an Arch container with `makepkg`, lint with
  `namcap`, install it, run `scoot --version` and the headless smoke test
  (`scripts/smoke-test.sh`) against the installed binary. That is the proof the
  package works.
- `aarch64`: Arch Linux ARM is a separate ecosystem; decide whether to support it.

## Done when

`makepkg` builds each package in a clean container, `namcap` is clean, and the
smoke test passes against the installed files.

## Resolved 2026-10-08 (PR #517)

What landed, item by item:

- **Six PKGBUILDs** under `packaging/arch/`: `scoot`, `scootbg`,
  `scootbar` release builds from the tagged tarballs (the trio from
  `scoot-v$pkgver`, the bar from its own `scootbar-v$pkgver`) plus
  `-git` twins tracking `main` (`pkgver()` counts commits past the
  crate version: `0.1.0.r1794.gde065f8e` on the day). No `scootctl`
  package: it is the client library inside the `scoot` binary
  (`scoot msg`). The coupled pair is a versioned dependency
  (`scoot` needs exactly `scootbg=$pkgver`); the bar depends on no
  compositor package (standard Wayland protocols only). Separate
  PKGBUILDs rather than one split package, because a split package
  forces one `pkgver` on members whose versions move independently.
- **Dependencies from data**: the `ldd` closure of an aarch64 release
  build (`scoot` 6.5 MB: exactly `libinput.so.10`, `libseat.so.1`,
  `libudev.so.1`, `libpixman-1.so.0`, `libxkbcommon.so.0`;
  `scootbg` 1.6 MB and `scootbar` 2.4 MB libc-only), mapped to Arch
  names verified against the package indexes on both architectures:
  `seatd` (not `libseat`: renamed, `Provides: libseat.so`), `sh`
  (the `scoot-session` launcher; `bash` provides it), `libgcc` (the
  detected provider of `libgcc_s`), `systemd-libs` for `libudev`.
  `mesa`/`libdrm` are `optdepends` for a `gpu-scanout` rebuild; the
  `xwayland` tier needs only the `Xwayland` binary at run time.
- **Installed files**: `/usr/bin/scoot-session` plus
  `/usr/share/wayland-sessions/scoot.desktop` (with `DesktopNames`,
  the session's XDG_CURRENT_DESKTOP name, as sway and the Nix-built
  entry carry), licenses under `/usr/share/licenses/<pkg>` including
  the `cargo deny list` inventory generated at build time, and
  `scoot --print-default-config` output as the `scoot` example (it
  documents the `[wallpaper]` handoff, so `scootbg` ships no separate
  one). Nothing is written to a user's home; uninstall is clean.
- **Bar restart**: `scootbar.service` user unit (`Restart=on-failure`,
  `RestartSec=2`, `KillMode=process`, bound to the shared
  `graphical-session.target`), the same policy as
  `nix/modules/scootbar-home.nix`, with install notes in a
  `.install` file.
- **CI** (`.github/workflows/arch.yml`, x86_64): all six with
  `makepkg` in `archlinux:base`, `namcap` on the PKGBUILDs and the
  built packages (errors fail the run; the surviving warnings are
  each explained in `packaging/arch/README.md`), `.SRCINFO`
  freshness against `makepkg --printsrcinfo`, `pkgver`-vs-manifest
  consistency, install of the release trio and the `-git` trio in
  turn with every `--version`, the bar's `--check` on the installed
  example, and the headless smoke test against the installed
  binaries. The release tarball URLs substitute same-layout HEAD
  tarballs until the first tag is cut (only URL-plus-checksum is
  substitution; `sha256sums` are `SKIP` until `updpkgsums` runs at
  release time). `packaging/arch` is a no-op in ci.yml's classify, so
  PKGBUILD-only changes run there alone.
- **`aarch64`**: included. Arch Linux ARM carries the same names for
  every dependency (`seatd`, `cargo-deny` checked against its index),
  and the tree already ships `aarch64-linux` release binaries from
  the same source. CI builds x86_64 only (the `archlinux` image ships
  no ARM variant); the entry is one line to drop if it misbehaves.
- **Docs**: the site releases page Arch line now says `seatd`
  (`pacman -S libseat` no longer resolves), and the packaging page
  points at the landed `packaging/arch/` tree.
- **Found along the way** (the mandated `actionlint` run on the
  touched workflows): ci.yml published `docs-site` from
  `steps.classify.outputs.docs-site`, which the step never writes,
  while the job read `needs.changes.outputs.docs_site`, which the job
  never publishes — both hops empty, so the docs-site job ran on
  every PR. Gated for real now, in the same PR.

Evidence: PR #517 green (arch job 12m6s: six `makepkg` builds,
`namcap` logs in the `arch-packages` artifact, both trios installed
with `--version`, `--check ok` on the installed example, two
headless smoke runs green); `actionlint` clean on both workflows;
`nix flake show --all-systems`, `nix build .#docs-site`,
`cargo deny check` green on the M2; `scripts/backlog check` shows
only the 3 known pre-existing problems. No AUR push, no tag, no
release anywhere (per the brief).

Deliberate deviations, each justified above: separate PKGBUILDs, not
one split package (independent versions); `options=('!debug')`
(namcap reads the auto-split debug symlink as dangling);
`-git` licenses under their own name (namcap requires the pkgname
dir); `SKIP` checksums until the first tag exists to hash.
