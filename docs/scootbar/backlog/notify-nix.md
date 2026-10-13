---
title: "scootnotify: Nix modules, the look layer, the opt-in desktop profile switch"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-bar-feed"
milestone: "M7"
---

# scootnotify: Nix and the look layer

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#stylix-and-look-integration)
and the [theming contract](../../../dev/design/shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)
(live looks, PR #578). Serves **daily-drive**.

## What to do

- **`programs.scootnotify`** (home-manager and NixOS halves, the scootbar layout). The
  rendered file holds only the user's `settings`; no look or Stylix value is merged into it.
- **The look-layer loader in the crate** (if `notify-popups` did not already land it):
  `$XDG_STATE_HOME/scoot/look/notify.toml` under the user's file per key, the allowlist,
  the contract's string grammar, a dangling link meaning no layer, and `msg reload` re-reading
  both.
- **The user unit**:
  - `X-Reload-Triggers` and `ExecReload` (home-manager at the pinned `efa3ccb` has
    `X-Reload-Triggers`, `modules/systemd.nix:140`; check what NixOS switching does to a
    user unit before choosing its trigger there);
  - `RestartPreventExitStatus=3`;
  - the D-Bus activation file.
- **`desktop.notifications.daemon`** widens to `"scootnotify"`, and the default stays
  `"mako"`. With it: no mako, no feed script, the three binds and the bar's `on-click`
  on `scootnotify msg`, and the eval warning for mako-shaped `notifications.settings`.
- **Flake outputs.**

Stylix reaches the app only as the `stylix` look (#578's `look-stylix-source`). This ticket
reads no Stylix option. If the contract's D5 is decided the other way, it carries scootbar's
per-leaf ladder instead ([Decisions needed 10](../../../dev/design/shell-apps.md#decisions-needed)).

## Acceptance

- `nix/scootnotify-tests.nix` pins:
  - the rendered file equals the user's `settings`;
  - the unit fields;
  - the profile switch both ways, with mako's output byte-identical under the default;
  - never both activation files installed;
  - `--check` on rendered files.
- Crate tests for the merge, the allowlist, the grammar and the dangling link.
- A real-login proof on the M2 with the switch on: a `scoot look apply` swap reloads the
  daemon and an on-screen card survives it, recolored.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The M2 screenshots per shipped look before and after a swap, and the unit's journal across it.

## Not in this ticket

Flipping the default (`notify-default-flip`); distro packages; the contract's own tickets.
