---
title: "scootlaunch: Nix module, the look layer, the opt-in launcher and picker switch"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-dmenu, launch-icons"
milestone: "M7"
---

# scootlaunch: Nix and the look layer

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#stylix-and-look-integration) and the
[theming contract](../../../dev/design/shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)
(live looks, PR #578). Serves **daily-drive**.

## What to do

- **`programs.scootlaunch`** (Home Manager and NixOS). The rendered `launch.toml` holds only
  the user's `settings`.
- **`desktop.launcher.daemon`** widens to `"scootlaunch"`, and the default stays `"fuzzel"`.
  With it:
  - `scoot-launcher` execs `scootlaunch "$@"` without color flags;
  - the pickers' and choosers' menu becomes `scootlaunch --dmenu`.

Stylix reaches it only as the `stylix` look.

## Acceptance

- `nix/scootlaunch-tests.nix` pins:
  - the rendered file equals the user's `settings`;
  - fuzzel's output is byte-identical under the default;
  - every flag every call site passes is in the supported set;
  - `--check`.
- An M2 real-login proof per look.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip, distro packages.
