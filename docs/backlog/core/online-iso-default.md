---
title: "Ship the slim online ISO as the default, offline as a build-it-yourself option"
status: "open"
area: "core"
priority: "medium"
blocked: "maintainer decision: the stock-parity online ISO is over GitHub's 2 GiB release-asset limit"
---

# Ship the slim online ISO as the default, offline as a build-it-yourself option

Filed 2026-10-28 on the maintainer's decision ("Online default, offline as
an option you can build with the command script to make the iso. That way we
can ship the online installer with instructions for building the offline
one"). Serves **daily-drive**: the ISO is the try-it-on-hardware route, and
it should match the stock NixOS graphical installer's hardware and feature
support while staying a small download.

## The gap

The graphical installer already needs the network for any real install
(Calamares refuses to leave its first page without it), so the reference
target closure and flake input sources the ISO carried helped only the QEMU
test's canonical offline install and a local-store speedup. The 2 GB cut set
(speech synthesis off, firmware trims, xz squashfs) existed only to fit that
closure under GitHub's release-asset cap.

## What to do

scoot-iso#21 splits the flake: `iso` (online, the default and the release)
and `iso-offline` (adds the reference closure, built locally with
`scripts/build-iso-docker.sh --offline`, never released), reverts the cuts so
both match stock parity, and gives the QEMU test an online mode that installs
the canonical system with the network up. The site install page points at the
release's online ISO and gives the offline build command.

## Open: the online ISO does not fit a GitHub release

Measured on scoot-iso#21's code tree (scoot v0.6.10; CI runs 40699628967 and 40707845699 for x86_64, an aarch64 machine for aarch64), no trims:

- online x86_64 2,729,345,024 bytes, aarch64 2,729,836,544 bytes, over the
  2,147,483,648-byte (2 GiB) asset limit by about 580 MB;
- offline x86_64 3,134,275,584 bytes, aarch64 3,130,519,552 bytes.

aarch64 savings from each former cut, applied to the online ISO alone (the
x86_64 savings are unmeasured, and x86_64 is the only arch the release job
builds; the firmware cut differs by arch, x86_64 dropped all of `qcom`):
speech synthesis off 242,180,096; xz squashfs 161,869,824; firmware trims
427,192,320; speech plus firmware 669,368,320 (2,060,468,224 bytes, under
the limit); all three 814,505,984 (1,915,330,560 bytes). The release job
refuses an ISO over the limit, so no release can attach the stock-parity
image. The choice is the maintainer's: host the online ISO outside GitHub
releases, restore the smallest set of cuts, or another route. The CI size
gate in scoot-iso#21 (2,800,000,000 bytes) assumes the first; the second
needs it back under 2 GiB.

## Not in this ticket

Publishing an ISO release; per-arch aarch64 builds in CI.
