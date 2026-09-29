---
title: "`scootlaunch`: the launcher, and a dmenu mode the bar's pickers can use"
status: "open"
area: "scootbar"
priority: "low"
blocked: "extract-scootui"
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
  keyboard path (no IME in v1, stated).
- **A dmenu mode**: lines on stdin, the selection on stdout. This is what lets
  the bar's pickers (WiFi networks, power menu, audio sinks) work before
  [popups](popups.md) exist, with no picker code in the bar.
- An application list from `.desktop` entries. Parsing them at every open is
  the cost to measure against caching; the launcher must start and show the
  first frame fast, and hold nothing when closed.
- Ranking by recent use in a tiny state file.

Same standards: lowest resource use of its class, a release gate against
`fuzzel`, `wofi`, `tofi` and `bemenu`, docs in the same PR, a Nix module.
