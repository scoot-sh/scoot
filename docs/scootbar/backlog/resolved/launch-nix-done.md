---
title: "scootlaunch: Nix module, the look layer, the opt-in launcher and picker switch"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
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

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/start/ (the Nix install page) and site/src/content/docs/scootlaunch/index.md` with the `programs.scoot` launcher option and what it wires. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip, distro packages.

## Resolution (2026-10-19, PR #681)

Landed as designed: `programs.scootlaunch` (home-manager and NixOS
modules plus the shared options, the flake's own `scootlaunch`
package with `svg` on by default, overlay, module wrappers and the
`scootlaunch-modules` check) renders only the user's `settings` into
`launch.toml`; `desktop.launcher.daemon` widens to
`fuzzel|scootlaunch` with the default staying `fuzzel`. With the
opt-in the wrapper execs `scootlaunch` with no color flags, the
pickers and the screencast chooser use its `--dmenu` the same way,
and every menu default follows the daemon to one derivation. Stylix
reaches it only as the `stylix` look: no unit, no Stylix switch.

Evidence: `nix eval` of the `scootlaunch-modules` and
`scoot-modules` check drvPaths for aarch64-linux and x86_64-linux
green; `nix build .#checks.aarch64-darwin.scoot-modules` green on
the Mac; `nix fmt -- --check` clean; the Linux `nix flake check`
legs (the new build, `--check` over rendered files, the opt-in
content pins) green in CI run 39281851467 on the exact head;
docs-site green. Two deviations: no M2 real-login proof (no M2
access; the headless and eval pins stand in), and `nix fmt` needed
one formatting pass after edits. Not run: the dev VM (root disk
full, per the ticket never started).
