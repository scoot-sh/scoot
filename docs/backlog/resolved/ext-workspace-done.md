---
title: "`ext-workspace-v1` protocol support \u2014 DONE as item 15."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `ext-workspace-v1` protocol support — DONE as item 15.

~~`ext-workspace-v1` protocol support~~ — DONE as item 15. Original
entry, left as written: `flexwm-core` already has a real
workspace model (`Output::workspaces`, `active_workspace`,
`FocusWorkspace`/`MoveWindowToWorkspace` actions in `world/mod.rs` and
`world/actions.rs`) — it's just not exposed outside keybindings/IPC
actions today. This protocol (the modern, compositor-agnostic successor
to the various one-off wlr workspace protocols) would let external tools
(bars, workspace switchers/indicators) query and switch workspaces the
same way sway/Hyprland's bars do. Unlike layer-shell, the pinned Smithay
rev has no existing helper for this protocol at all (checked: no
`ext_workspace` reference anywhere in the pinned checkout) — the global,
object lifecycle, and event plumbing would need to be implemented
directly against `wayland-server`, not layered on a Smithay helper.
Depends on layer-shell landing first in practice, since the main
consumers (bars) need both to be useful together. No design work done
yet.
