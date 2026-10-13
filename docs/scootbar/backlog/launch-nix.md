---
title: "scootlaunch: Nix module, Stylix and look rungs, the opt-in launcher and picker switch"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-dmenu"
milestone: "M7"
---

# scootlaunch: Nix and Stylix

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#stylix-and-look-integration). Serves
**daily-drive**.

## What to do

`programs.scootlaunch` (home-manager and NixOS), the Stylix mapping (`selection` = `base02`,
`match` = `base0A`, `sansSerif`, `sizes.popups`, `opacity.popups`) and the look mapping equal to
what `fuzzel-theme.nix` passes today, per leaf. `desktop.launcher.daemon` widens to
`"scootlaunch"` (default stays `"fuzzel"`): `scoot-launcher` execs `scootlaunch "$@"` without
color flags, the pickers' and choosers' menu becomes `scootlaunch --dmenu`. Each look gains
`launch.toml`.

## Acceptance

`nix/scootlaunch-tests.nix` pins the mappings, user wins, `--check`, the fuzzel output
byte-identical with the default, and an eval pin that every flag every call site passes is in
the supported set. An M2 real-login proof per look.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip, distro packages.
