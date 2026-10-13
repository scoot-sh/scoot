---
title: "scootlaunch: the dmenu contract (fuzzel's flags the call sites use) and run mode"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-ui"
milestone: "M7"
---

# scootlaunch: dmenu and run

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#the-dmenu-contract). Serves
**daily-drive** (the bar's WiFi and Bluetooth pickers, the clipboard and capture choosers).

## What to do

`--dmenu` with every flag in the design's table at fuzzel's meaning, `--index`,
`--placeholder`, the stdin bounds, exit 1 on Escape, a loud exit 2 on any unsupported flag;
`--run` / `--list-executables-in-path`.

## Acceptance

Each repo call site (`apps-network-pick.sh`, `apps-bluetooth-pick.sh`, `keys-home.nix`
clipboard, `capture-home.nix`) run against scootlaunch in a headless test with the same
flags; the WiFi password prompt masked; a 65,536-line stdin bounded.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Swapping the call sites in Nix (`launch-nix`).
