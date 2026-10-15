---
title: "Live-swappable looks across the scoot desktop (epic)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Live-swappable looks across the scoot desktop (epic)

Filed 2026-10-12 from the maintainer's question: "Are we able to live swap
looks? I was thinking it'd be really cool to build out looks and be able to
swap between them, maybe even with a tool." Serves **daily-drive** first (a
desktop whose look you change in a second, and a format that makes new looks
cheap) and **computer use** second (an agent or a VM test can pin a look over
IPC and screenshot it).

The design, the liveness evidence per component, the single published token
list and the rejected alternatives are in [the design doc](../../../dev/research/live-looks-design.md). Every
child implements against it; read it first. Its section 9 is also the
theming contract of the shell-apps plan (PR #579).

## The gap

A look is chosen when Nix evaluates the configuration and nothing switches it
afterwards: the registry is Nix data (`nix/modules/desktop.nix:1931-2147`),
its values are merged into the compositor's and bar's TOML at module
priorities (`home.nix:532-539`, `scootbar.nix:374-378`) or baked into store
paths (`idle-home.nix:83`, `keys-home.nix:72-79`), and `install.sh --look`
copies whole files once (`site/public/install.sh:411-462`). Yet the
first-party apps can already change appearance live (`reload.rs:23-33`,
scootbar `msg reload`, `scootbg apply-config`).

## What to do (the architecture, one paragraph)

A source look is a directory with a `look.toml` of tokens; one pure crate,
`scoot-look`, parses, validates (keys *and* values) and renders per-app
fragments into a separate rendered look. A per-session state
(`<state>/look.json` plus a link to a rendered look) selects the active one;
scoot and scootbar merge that layer under the user's own file on raw keys,
per key, with `[wallpaper]` as the one whole unit and a derived key
following the layer that supplied its base. `scoot look apply`
renders and validates, records and repoints, then drives each app's
control-socket `reload` *scoped to the look* (the user's pending edits are
never applied by a swap), in order, under one lock, rolling back on a
refusal and naming every shadowed key. A look is presentation only. Stylix
stops being a precedence layer and becomes a synthesized `stylix` look. Nix
renders the catalog; the live choice is state Nix does not own, under a
stated rule.

## Children, in order

| Slug | Priority | Serves | Blocked by |
|---|---|---|---|
| [`look-format`](look-format.md) | high | both | |
| [`look-shipped`](look-shipped.md) | high | both | `look-format` |
| [`look-layer-scoot`](look-layer-scoot.md) | high | both | `look-format` |
| [`look-scoped-reload`](../resolved/look-scoped-reload-done.md) | high | both | `look-layer-scoot` |
| [`look-session-state`](../resolved/look-session-state-done.md) | medium | computer use | `look-layer-scoot` |
| [`look-layer-scootbar`](../../scootbar/backlog/resolved/look-layer-scootbar-done.md) | high | daily-drive | `look-format` |
| [`scootbar-style-reload`](../../scootbar/backlog/resolved/scootbar-style-reload-done.md) | medium | daily-drive | |
| [`look-apply-cli`](../resolved/look-apply-cli-done.md) | high | both | `look-shipped`, `look-scoped-reload` |
| [`look-cycle-preview`](look-cycle-preview.md) | medium | daily-drive | `look-apply-cli` |
| [`look-ipc`](../ipc/look-ipc.md) | medium | computer use | `look-apply-cli` |
| [`look-golden-screenshots`](../testing/look-golden-screenshots.md) | medium | computer use | `look-ipc` |
| [`look-nix-catalog`](../packaging/look-nix-catalog.md) | high | daily-drive | `look-apply-cli` |
| [`look-import-base16`](look-import-base16.md) | medium | daily-drive | `look-format` |
| [`look-stylix-source`](../packaging/look-stylix-source.md) | high | daily-drive | `look-nix-catalog`, `look-import-base16` |
| [`look-toolkit-live`](../resolved/look-toolkit-live-done.md) | medium | daily-drive | `look-apply-cli` |
| [`look-leaves-daemons`](../packaging/look-leaves-daemons.md) | medium | daily-drive | `look-apply-cli` |
| [`look-leaves-launchers`](../packaging/look-leaves-launchers.md) | medium | daily-drive | `look-apply-cli` |
| [`look-leaves-terminal`](../packaging/look-leaves-terminal.md) | medium | daily-drive | `look-apply-cli` |
| [`look-greeter`](../packaging/look-greeter.md) | low | daily-drive | `look-nix-catalog` |
| [`look-nix-retire-registry`](../packaging/look-nix-retire-registry.md) | medium | daily-drive | the Nix, Stylix, toolkit, leaves and greeter children |
| [`look-picker`](../../scootbar/backlog/look-picker.md) | medium | daily-drive | `look-apply-cli` |
| [`look-authoring`](look-authoring.md) | medium | daily-drive | `look-apply-cli` |
| [`look-install`](look-install.md) | low | daily-drive | `look-authoring` |
| [`look-install-sh`](../packaging/look-install-sh.md) | medium | daily-drive | `look-apply-cli` |
| [`look-share`](look-share.md) (later) | low | daily-drive | `look-install` |
| [`look-auto-switch`](look-auto-switch.md) (later) | low | daily-drive | `look-toolkit-live` |

**The minimum slice** is `look-format` → `look-shipped`, `look-layer-scoot`
→ `look-scoped-reload`, and `look-layer-scootbar` → `look-apply-cli`:
non-Nix, compositor, bar and wallpaper. It cannot be
dogfooded on a home-manager machine as is, because Nix writes look and
Stylix values into the user's own files and those shadow every look; the
design doc's section 10 gives the dogfood settings, and `look-apply-cli`
documents them.

## Decisions the maintainer should make first

In the design doc's "Decisions needed": D2 (`programs.scoot.look`), D3 (no
module lists), D4 (default catalog), D5 (Stylix as a source), D7 (no gap in
v1), D9 (the shell-apps plan follows section 9), D10 (per display vs per
user), D11 (dconf ownership), D12 (no git install in v1), D13 (font
carriers), D14 (Stylix targets keep their paths), D15 (the shipped looks
carry the examples' shape and opacity, a visible change for every Nix
profile user). D1, D6 and D8 are defaults that need no call.

## Maintainer direction, 2026-10-14

- **Works on Nix and without Nix, equally.** No core behavior may need Nix.
- **Stylix is a source, never a requirement.** It is a Nix-only way to get a
  palette. Every theming path a Stylix user has must also exist for a plain
  install, using `scoot-look` alone: the `look-leaves-*` and
  `look-toolkit-live` tickets render each third-party app's own config (foot,
  mako, fuzzel, GTK `gtk.css`, qt6ct colors, dconf keys through the
  `gsettings`/`dconf` CLI) from the look's tokens **once, at `scoot look
  apply`**, then send that app's own reload signal. No resident theming
  daemon, no watcher process, no per-frame work: memory and CPU cost is zero
  between swaps. `look-import-base16` gives a plain install any base16 scheme
  as a source look, which is what Stylix gives a Nix user.
- **Own launcher and notification daemon are not prerequisites.** fuzzel
  (reads its config on every launch, so it is live by construction) and mako
  (`makoctl reload`) are themed through the render path above. `scootlaunch`
  and `scootnotify` stay separate, later work; if a leaf proves unthemeable
  live, that is the trigger to build the first-party one.

## Not in this epic

Per-window looks, per-output colors, a high-contrast look, a live login
screen, and live colors in already-running GTK, Qt and terminal processes
beyond what their own mechanisms allow.
