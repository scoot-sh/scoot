---
title: "Arch Linux packages: PKGBUILDs (release and `-git`), AUR, built and linted in CI"
status: "open"
area: "packaging"
priority: "low"
blocked: "release-artifacts"
---

# Arch Linux packages

Filed 2026-09-29. Serves **daily-drive**. Arch is the smallest packaging
target (a PKGBUILD is one file), so it goes first of the three.

## What to build

- **Split packages**, matching the independent versions
  ([independent-versioning](../resolved/independent-versioning-done.md)): `scoot`, `scootctl`,
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
