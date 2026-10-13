---
title: "scootnotify: Nix modules, Stylix and look rungs, the opt-in desktop profile switch"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-bar-feed"
milestone: "M7"
---

# scootnotify: Nix and Stylix

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#stylix-and-look-integration) and the
[shared rules](../../../dev/design/shell-apps.md#theme-tokens-stylix-and-looks). Serves **daily-drive**.

## What to do

`programs.scootnotify` (home-manager and NixOS halves, the scootbar layout), the
Stylix mapping (`base00/05/03/0D/08/02`, `sansSerif`, `sizes.popups`, `opacity.popups`)
and the look mapping per leaf at the documented priorities, the user unit with
`X-Reload-Triggers` and `ExecReload` (home-manager at the pinned `efa3ccb` has
`X-Reload-Triggers`, `modules/systemd.nix:140`; check what NixOS switching does to a user
unit before choosing its trigger there), `RestartPreventExitStatus=3`,
the D-Bus activation file. `desktop.notifications.daemon` widens to `"scootnotify"` (default
stays `"mako"`): no mako, no feed script, the three binds and the bar's `on-click` on
`scootnotify msg`, the eval warning for mako-shaped `notifications.settings`. Each look
directory gains `notify.toml`. Flake outputs.

## Acceptance

`nix/scootnotify-tests.nix` pins every mapping, per-leaf user wins, `--check` on rendered
files, the profile switch both ways (mako output byte-identical with the default), and never
both activation files installed. The real-Stylix script gains a block. A real-login proof on
the M2 with the switch on: a look change reloads without losing an on-screen card.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The M2 screenshots per look, and the unit's journal across the reload.

## Not in this ticket

Flipping the default (`notify-default-flip`), distro packages.
