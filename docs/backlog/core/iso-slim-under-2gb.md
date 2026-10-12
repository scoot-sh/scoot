---
title: "Slim the scoot ISO under GitHub's 2 GB release-asset cap"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
