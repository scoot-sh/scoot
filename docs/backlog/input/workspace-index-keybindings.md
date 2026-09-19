---
title: "Workspace shortcuts: no default bind for a numbered workspace, and no move-to-index action at all"
status: "open"
area: "input"
priority: "medium"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block"
---

# Workspace shortcuts: no default bind for a numbered workspace, and no move-to-index action at all

Requested 2026-09-19. Two separate gaps that look like one, and only the
first is a keybinding change.

## What exists today

- `focus-workspace up|down` and `move-window-to-workspace up|down`, bound by
  default to `Super+Ctrl+j`/`k` and `Super+Ctrl+Shift+j`/`k`.
- `focus-workspace-index N` — a real action in `scoot-core`, reachable over
  IPC (`crates/scoot-ipc/src/action.rs:53`) and bindable from the config
  file, but **not bound by default**.

## Gap 1 — no `Super+1`..`Super+9` (keybinding only)

Jumping straight to a numbered workspace is the near-universal convention
(i3, sway, niri, Hyprland all ship it), and scoot has the action already —
it just isn't in the default set, so a new user gets relative up/down
navigation and has to discover `focus-workspace-index` in the config
reference to get the behaviour they expect.

Cheap: default binds for the action that already exists. The only real
question is how many to ship (1–9 is conventional; scoot's workspace set is
dynamic, so decide what a bind for a workspace that doesn't exist yet does —
create it, clamp, or no-op).

## Gap 2 — there is no `move-window-to-workspace-index` (missing action)

This one is not a binding gap. `move-window-to-workspace` takes **only**
`up|down`; there is no index form anywhere — not in the core, not over IPC,
not in the config grammar. So you can *focus* workspace 5 directly but
cannot *send the focused window* to workspace 5 directly; you have to step
it one workspace at a time.

That asymmetry is worth fixing regardless of the keybinding question,
because it is also a gap for the agent-driven use case: an agent placing a
window has the same one-step-at-a-time problem, and stepping is not
idempotent in the way an absolute target is.

Shape: mirror `FocusWorkspaceIndex` — core action, `scoot-ipc` variant and
conversion, config-file grammar, then the default bind
(`Super+Shift+1`..`9`, matching the relative pair's `Shift` convention).
The IPC wire format is additive, so no version bump.

## Not in scope

Naming workspaces, or binding by name rather than index. Different feature,
and worth its own decision about whether the workspace set is dynamic or
declared.
