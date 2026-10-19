---
title: "`scootlaunch`: the launcher, and a dmenu mode the bar's pickers can use (epic)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-agent-json, launch-windows-mode, launch-packaging, launch-default-flip"
milestone: "M7"
---

# `scootlaunch`

Filed 2026-09-29 as a pointer, not a spec: it gets its own backlog and design
pass when it starts. Serves **daily-drive**.

## The plan (2026-10-12)

The design is [dev/design/scootlaunch.md](../../../dev/design/scootlaunch.md); shared parts are in
[shell-apps.md](../../../dev/design/shell-apps.md). The name stays `scootlaunch`, though the request said
"scootlauncher" ([why](../../../dev/design/shell-apps.md#names)).

Children, in order:

1. [launch-prototype](resolved/launch-prototype-done.md): the go/no-go gate against fuzzel, out of the
   workspace.
2. [launch-desktop-index](resolved/launch-desktop-index-done.md), then [launch-match-rank](resolved/launch-match-rank-done.md).
3. [launch-ui](resolved/launch-ui-done.md), then [launch-spawn](launch-spawn.md) and [launch-icons](resolved/launch-icons-done.md).
4. [launch-dmenu](launch-dmenu.md) and [launch-agent-json](resolved/launch-agent-json-done.md).
3. [launch-ui](resolved/launch-ui-done.md), then [launch-spawn](launch-spawn.md) and [launch-icons](resolved/launch-icons-done.md).
4. [launch-dmenu](launch-dmenu.md) and [launch-agent-json](resolved/launch-agent-json-done.md).
5. [launch-nix](resolved/launch-nix-done.md), [launch-packaging](launch-packaging.md),
   [launch-default-flip](launch-default-flip.md); later, [launch-windows-mode](launch-windows-mode.md).

Compositor side: [layer-shell-default-output](../../backlog/core/layer-shell-default-output.md).
Resource tables are published here per milestone, against fuzzel, tofi, bemenu and wofi.

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

Theming contract (added 2026-10-12): read the look layer at launch, never
Stylix or a palette of its own; see the [live looks design](../../../dev/research/live-looks-design.md),
section 9. A `Look:` list through the dmenu mode is an optional hook
([`look-picker`](resolved/look-picker-done.md)).
