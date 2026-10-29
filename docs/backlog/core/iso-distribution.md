---
title: "Distribute the scoot ISO: the online ISO is over GitHub's 2 GiB release-asset cap"
status: "open"
area: "core"
priority: "low"
blocked: "maintainer decision (2026-10-28: skip distribution for now, keep the builder)"
---

# Distribute the scoot ISO: the online ISO is over GitHub's 2 GiB release-asset cap

Filed 2026-10-28; **parked** on the maintainer's decision ("Skip the iso
distribution for now. Keep the builder."). Serves **daily-drive**: a
downloadable ISO is the easy try-it-on-hardware route. Today nothing is
published; users build the ISO ([online-iso-default](../resolved/online-iso-default-done.md)).

## The gap

The untrimmed online ISO measures 2,729,345,024 bytes on x86_64 (CI) and
2,729,836,544 on aarch64. GitHub rejects release assets of 2 GiB
(2,147,483,648) or more, so it is about 582 MB over. The offline ISO is
3,134,275,584 bytes (x86_64) and is built locally anyway.

Where the bytes go (aarch64, scoot-iso 39a689a, per-path compressed
sizes from a whole-image squashfs analysis): the ISO is about 97% one
squashfs (2,620,985,344 bytes) plus the raw kernel, initrd and boot files
(about 109 MB). `linux-firmware` is a third of it (about 863 MB, already
compressed, so no squashfs setting shrinks it); speech synthesis is about
10% (mbrola voices, flite, espeak); mesa plus llvm 6%; firefox 6%; the
installer's Qt and Calamares 5%; fonts 5%.

## Options, measured (aarch64; whole-image squashfs, then ISO = squashfs + 108,851,200)

All of these are live-image-only: the installed system comes from
`iso/target/*` through nixos-install and is unaffected.

| Option | Saves (squashfs bytes) | ISO bytes | vs 2 GiB |
|---|---:|---:|---|
| baseline, no trims | | 2,729,836,544 | 582 MB over |
| speech off in the live image | 242,180,096 | | |
| xz squashfs | 169,378,304 | 2,560,458,752 | over |
| firmware: drop mellanox, prestera, qcom | 397,520,896 | | |
| firmware: same plus nvidia | 506,019,840 | 2,223,816,704 | 76 MB over |
| firmware (nvidia kept) + speech off, computed from the two deltas | about 640 MB | about 2,090,000,000 | about 57 MB under |
| firmware (nvidia dropped) + speech off, computed | about 748 MB | about 1,981,000,000 | about 166 MB under (and under 2.0 GB) |
| the previous plus docs, nixpkgs registry copy and mesa-demos off (built) | 845 MB | 1,884,688,384 | 263 MB under |
| speech off + the #20 firmware set (built on the online ISO) | | 2,060,468,224 | 87 MB under |

Live-side costs of the cuts: no text-to-speech in the live session; no
firmware on live boot for Qualcomm SoC, Mellanox/Prestera and (with the
nvidia cut) Nvidia GSP, so those GPUs fall back to the firmware framebuffer
through scoot's CPU path; no man pages or `nixos-help`; `nix shell nixpkgs#x`
needs the network. x86_64 numbers are unmeasured: it drops the whole `qcom`
directory, so its firmware savings are larger.

Other routes, none built: split the ISO into two release assets (needs a
rejoin step for users); host it outside GitHub releases (a static file
host or torrent; trust anchor `SHA256SUMS`).

## What to do

The maintainer picks a route. Whichever it is, re-measure on x86_64 in CI
(the only arch CI builds), restore a release job (publish the online ISO
only, `SHA256SUMS` beside it, never the offline ISO) and re-gate its size
under whatever the chosen route allows. The measurements above come from
a one-off analysis (2026-10-28) whose harness is not in the repo.

## Not in this ticket

The builder, both variants and the CI that boot-tests them
([online-iso-default](../resolved/online-iso-default-done.md)).
