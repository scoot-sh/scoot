---
title: "scootnotify: Nix modules, the look layer, the opt-in desktop profile switch"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-20"
---

# scootnotify: Nix and the look layer

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#stylix-and-look-integration) and the
[theming contract](../../../dev/design/shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)
(live looks, PR #578). Serves **daily-drive**.

## What to do

- **`programs.scootnotify`** (Home Manager and NixOS halves, the scootbar layout). The rendered
  file holds only the user's `settings`; no look or Stylix value is merged into it (#578 §9
  item 4).
- **The user unit**:
  - `X-Reload-Triggers` and `ExecReload`. Home Manager at the pinned `efa3ccb` has
    `X-Reload-Triggers` (`modules/systemd.nix:140`); check what NixOS switching does to a user
    unit before choosing its trigger there.
  - `RestartPreventExitStatus=3`.
  - The D-Bus activation file.
- **`desktop.notifications.daemon`** widens to `"scootnotify"`, and the default stays `"mako"`.
  With it: no mako, no feed script, the three binds and the bar's `on-click` on
  `scootnotify msg`, and an eval warning for mako-shaped `notifications.settings`.
- **Flake outputs.**

Stylix reaches the app only as the `stylix` look (#578's `look-stylix-source`); this ticket reads
no Stylix option.

## Acceptance

`nix/scootnotify-tests.nix` pins:

- the rendered file equals the user's `settings`;
- the unit fields;
- the profile switch both ways, with mako's output byte-identical under the default;
- never both activation files;
- `--check` over the rendered files.

A real-login proof on the M2 with the switch on: a `scoot look apply` swap reloads the daemon,
and an on-screen card survives it, recolored.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/start/ (the Nix install page) and site/src/content/docs/scootnotify/index.md` with the `programs.scoot` notification option. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The M2 screenshots per shipped look before and after a swap, and the unit's journal across it.

## Not in this ticket

Flipping the default (`notify-default-flip`); distro packages; the contract's own tickets.

## Resolution (2026-10-20, PR #698)

Landed as designed: `programs.scootnotify` (home-manager and NixOS
modules plus the shared options, the flake's own `scootnotify`
package with no Cargo features today, overlay, module wrappers and
the `scootnotify-modules` check) renders only the user's `settings`
into `notify.toml`; `desktop.notifications.daemon` widens to
`mako|scootnotify` with the default staying `mako`. The user unit is
`Type=dbus` owning `org.freedesktop.Notifications`, reloads (never
restarts) on a config change, and stops instead of restart-looping on
the taken-name exit 3; the D-Bus activation file names the same
binary. With the opt-in, both package defaults follow the daemon
(the #681 lesson), mako and `scoot-notify-sync` stand down, the three
binds and the bar's on-click move onto `scootnotify msg`, and a
mako-shaped `notifications.settings` warns naming
`programs.scootnotify.settings`. Stylix reaches it only as the
`stylix` look.

Evidence: `nix eval` of the `scootnotify-modules` and `scoot-modules`
check drvPaths for aarch64-linux and x86_64-linux green on the Mac;
`nix build .#checks.aarch64-linux.scootnotify-modules`,
`.#checks.aarch64-linux.scoot-modules`, `.#checks.aarch64-linux.scootbar-modules`,
`.#checks.aarch64-darwin.scoot-modules` and `.#docs-site` green;
`nix fmt -- --check` clean; the full CI matrix green on the exact
head (run 39401007329, all 18 checks). Mutation-proved: reverting
`RestartPreventExitStatus` fails the unit pin; reverting the NixOS
package default fails the opt-in default pin. One deviation: no M2
real-login proof (no M2 access from here; the reload-keeps-cards
semantics are covered by the daemon's own control-socket tests) —
flagged for the reviewer/maintainer. Not run: the dev VM beyond
reachability (disk constraints per the ticket); all Linux builds ran
on the remote builder.
