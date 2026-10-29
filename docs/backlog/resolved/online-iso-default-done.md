---
title: "Online ISO as the default build, offline ISO as a build-it-yourself option"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
---

# Online ISO as the default build, offline ISO as a build-it-yourself option

Filed 2026-10-28 on the maintainer's decision ("Online default, offline as
an option you can build with the command script to make the iso"). Serves
**daily-drive**. Resolve this entry when scoot-sh/scoot-iso#21 merges.

## What landed (scoot-iso#21)

- Two flake outputs, both untrimmed (the stock NixOS graphical installer's
  hardware and feature support): `iso` (online, the default) and
  `iso-offline` (adds the reference target's closure and flake input
  sources, so the canonical system installs with the network cut).
- `scripts/build-iso-docker.sh` with `--offline` and `--check`;
  `scripts/qemu-test.sh --mode offline|online|live`.
- CI builds and boot-tests both: `build-iso`, `build-iso-offline`,
  `qemu-test` (offline install, network cut), `qemu-test-online` (asserts the
  reference closure is not on the ISO, installs with the network up, logs
  in). All green on x86_64 in scoot-iso run 40798966071 (head 14b034f).
- #20's cuts are reverted (speech synthesis off, trimmed firmware, xz, the
  2,000,000,000-byte gate), and `patch_consistency` fails if they come back.
- Measured: online 2,729,345,024 bytes (x86_64) / 2,729,836,544 (aarch64);
  offline 3,134,275,584 / 3,130,519,552.

## Not in this ticket

Publishing an ISO: parked as [iso-distribution](./iso-distribution.md). The
site install page says there is no downloadable ISO yet and gives the build
commands.
