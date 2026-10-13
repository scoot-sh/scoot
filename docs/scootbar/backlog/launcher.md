---
title: "`scootlaunch`: the launcher, and a dmenu mode the bar's pickers can use (epic)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-desktop-index, launch-match-rank, launch-ui, launch-dmenu, launch-agent-json, launch-windows-mode, launch-nix, launch-packaging, launch-default-flip"
milestone: "M7"
---

# `scootlaunch`

Filed 2026-09-29 as a pointer, not a spec: it gets its own backlog and design
pass when it starts. Serves **daily-drive**.

**Name: `scootlaunch`** (settled 2026-09-29, over `scootmenu`); the picker is
its `--dmenu` mode.

Its own small binary in the same shell family as the bar and
[`scootnotify`](scootnotify.md).

- An `overlay` layer surface with `keyboard_interactivity: exclusive`, which is
  what scoot already supports for launchers
  (`docs/protocols.md`, keyboard focus). It exists only while open.
- Drawn with [`scootui`](extract-scootui.md); text input through the normal
  keyboard path (no IME at first, stated).
- **A dmenu mode**: lines on stdin, the selection on stdout. This is what lets
  the bar's pickers (WiFi networks, power menu, audio sinks) work before
  [popups](resolved/popups-done.md) exist, with no picker code in the bar.
- An application list from `.desktop` entries. Parsing them at every open is
  the cost to measure against caching; the launcher must start and show the
  first frame fast, and hold nothing when closed.
- Ranking by recent use in a tiny state file.

Same standards: lowest resource use of its class, a resource ratchet against
`fuzzel`, `wofi`, `tofi` and `bemenu`, docs in the same PR, a Nix module.

## The plan (2026-10-12)

The design pass happened: [dev/design/scootlaunch.md](../../../dev/design/scootlaunch.md), with the parts it
shares in [shell-apps.md](../../../dev/design/shell-apps.md). The name stays `scootlaunch`; the request said
"scootlauncher" ([why](../../../dev/design/shell-apps.md#names)).

Children, in order (each `blocked` names its predecessors):

1. [launch-desktop-index](launch-desktop-index.md): parser, ids, locale, Exec, cache; `--list --json`.
2. [launch-match-rank](launch-match-rank.md): fuzzy and frecency; `--query --json`.
3. [launch-ui](launch-ui.md): the panel, launching (token, scope), icons; the go/no-go gate against fuzzel.
4. [launch-dmenu](launch-dmenu.md), [launch-agent-json](launch-agent-json.md),
   [launch-windows-mode](launch-windows-mode.md).
5. [launch-nix](launch-nix.md): module, Stylix, look; the opt-in launcher and picker switch.
6. [launch-packaging](launch-packaging.md), then [launch-default-flip](launch-default-flip.md).

Shared prerequisites: [scootui-entry-and-list](scootui-entry-and-list.md) (and through it the
keyboard spike and stack), and [scoot-dbus-extract](scoot-dbus-extract.md). On the compositor
side: [layer-shell-default-output](../../backlog/core/layer-shell-default-output.md).

## Resource ratchet

Published here per milestone until the app gets its own backlog, against fuzzel, tofi, bemenu
and wofi ([budgets](../../../dev/design/scootlaunch.md#budgets)).

## Theming

Through the live-looks contract only (PR #578; [how this plan maps onto it](../../../dev/design/shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)): the app reads the look layer under its own config, never Stylix.
