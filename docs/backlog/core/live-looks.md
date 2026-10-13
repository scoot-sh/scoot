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
desktop whose look you change in a second, and a format that makes new
looks cheap) and **computer use** second (an agent or a VM test can pin a
look over IPC and take deterministic screenshots of it).

The design, the liveness evidence per component and the rejected
alternatives are in [the design doc](../../../dev/research/live-looks-design.md). Every child implements
against it; read it first.

## The gap

A look is chosen when Nix evaluates the configuration and nothing switches
it afterwards: the registry is Nix data (`nix/modules/desktop.nix:1931-2147`),
its values are merged into the compositor's and bar's TOML at module
priorities (`home.nix:532-539`, `scootbar.nix:374-378`) or baked into store
paths (`idle-home.nix:83`, `keys-home.nix:72-79`), and `install.sh --look`
copies whole files once and never again (`site/public/install.sh:411-462`).
Yet the first-party apps can already change appearance live
(`reload.rs:23-33`, scootbar `msg reload`, `scootbg apply-config`).

## What to do (the architecture, one paragraph)

A look becomes a directory with a `look.toml` of tokens (palette roles,
fonts, shape, cursor, wallpaper, icons, night temperature, per-app
overrides), owned by one pure crate, `scoot-look`, that validates it and
renders a per-app fragment for every consumer. A mutable link,
`$XDG_STATE_HOME/scoot/look`, points at the active rendered look; scoot and
scootbar merge that layer under the user's own file per key, and per-launch
apps read it by path. `scoot look apply` renders and validates the target,
repoints the link atomically, then drives each app's existing control-socket
`reload` in order and rolls back on a refusal. A look is presentation only:
it may never carry a command. Stylix stops being a precedence layer and
becomes a synthesized `stylix` look. Home-manager renders the catalog into
the store; the live choice is state it does not own, under a stated rule.

## Children, in order

| Slug | Priority | Serves | Blocked by |
|---|---|---|---|
| [`look-format`](look-format.md) | high | both | |
| [`look-layer-scoot`](look-layer-scoot.md) | high | both | `look-format` |
| [`look-layer-scootbar`](../../scootbar/backlog/look-layer-scootbar.md) | high | daily-drive | `look-format` |
| [`look-apply-cli`](look-apply-cli.md) | high | both | `look-layer-scoot`, `look-layer-scootbar` |
| [`look-ipc`](../ipc/look-ipc.md) | high | computer use | `look-apply-cli` |
| [`look-golden-screenshots`](../testing/look-golden-screenshots.md) | medium | computer use | `look-ipc` |
| [`look-nix-catalog`](../packaging/look-nix-catalog.md) | high | daily-drive | `look-apply-cli` |
| [`look-import-base16`](look-import-base16.md) | medium | daily-drive | `look-format` |
| [`look-stylix-source`](../packaging/look-stylix-source.md) | high | daily-drive | `look-nix-catalog`, `look-import-base16` |
| [`look-toolkit-live`](../packaging/look-toolkit-live.md) | medium | daily-drive | `look-apply-cli` |
| [`look-app-leaves`](../packaging/look-app-leaves.md) | medium | daily-drive | `look-apply-cli` |
| [`look-picker`](../../scootbar/backlog/look-picker.md) | medium | daily-drive | `look-apply-cli` |
| [`look-authoring`](look-authoring.md) | medium | daily-drive | `look-apply-cli` |
| [`look-install-sh`](../packaging/look-install-sh.md) | medium | daily-drive | `look-apply-cli` |
| [`look-auto-switch`](look-auto-switch.md) | low | daily-drive | `look-toolkit-live` |
| [`look-greeter`](../packaging/look-greeter.md) | low | daily-drive | `look-nix-catalog` |

The first four are the smallest slice that proves the architecture end to
end on a non-Nix box with no Nix change: tokens, both look layers, and
`scoot look apply` swapping the ring, background, cursor, bar and wallpaper.

## Decisions the maintainer should make first

Listed with recommendations in the design doc's "Decisions needed": where
shipped looks live (`looks/` so CI runs on them), where `look` lives in Nix
(`programs.scoot.look`), whether a look may carry module lists (no),
Stylix's role (a source of a look), the default catalog, no central
registry, gap in a look, and the default keybinding.

## Not in this epic

Per-window looks, per-output colors, a live login screen, and live colors in
already-running GTK, Qt and terminal processes beyond what their own
mechanisms allow (the design doc's liveness table).
