---
title: "Slim the scoot ISO under GitHub's 2 GB release-asset cap"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
---

# Slim the scoot ISO under GitHub's 2 GB release-asset cap

Filed 2026-10-11. Serves **daily-drive**: the ISO is the planned first-class try-it-on-hardware route.

## The gap

The aarch64 ISO built from the `scoot-iso` flake is 3,041,998,848 bytes
(2.83 GiB), over GitHub's 2 GB release-asset cap. Releases would have to host
it elsewhere. Measured 2026-10-10 on the aarch64 `scoot-iso` build of scoot main at that date (no rev recorded; rebuild to confirm). Closure is 1,336 paths, about 6.88 GB NAR. Largest: linux-firmware
782 MiB, llvm lib 532, firefox 373, mesa 269, mbrola-voices 259, glibc-locales
222, nixos tarball 204, kernel 187.

## What to do

Start by dropping mbrola-voices, then weigh firmware, llvm and locales against
what the live image needs. Re-measure; aim under 2 GB, x86_64 and aarch64.

## Not in this ticket

Publishing the ISO. Nothing is released yet, and the docs show the ISO as
"coming" until the install guide is the easy one.

## Resolution

Resolved 2026-10-28 by scoot-sh/scoot-iso#20 (private repo). Remeasured at
scoot-iso main (a377694, scoot v0.6.9): aarch64 3,130,515,456 bytes, x86_64
about 3.07 GB (squashfs 3,002,442 KB, CI run 40566877896).

What landed, aarch64 bytes after each step: speech synthesis off, the
installed template included (645 MiB of mbrola voices; the live copy shared
files with the target's full set, so only removing both saves) plus
rack-switch firmware, 2,365,405,184; xz squashfs, 2,218,127,360; Qualcomm
SoC and NVIDIA GSP firmware dropped from the live image, 1,972,770,816. The
x86_64 ISO is 1,908,867,072 bytes (CI `build-iso`; `qemu-test` passed on the
same build). CI's `build-iso` now fails above 2,000,000,000 bytes; aarch64 is
not built by CI.

Trade-offs recorded in the PR: installed systems lose speech synthesis unless
`services.speechd.enable = true`; NVIDIA Turing-and-newer cards lose nouveau
on the live image. Locale trimming (`i18n.supportedLocales`) saves nothing:
calamares-nixos-extensions references the all-locales `glibc-locales`.

## Superseded

Superseded by scoot-sh/scoot-iso#21 (open), on the maintainer's decision of 2026-10-28:
no trims in either ISO, so both match the stock NixOS graphical installer.
It reverts the work above: the 2,000,000,000-byte gate and the cuts no longer exist there; see
[online-iso-default](./online-iso-default-done.md) and
[iso-distribution](../core/iso-distribution.md).
