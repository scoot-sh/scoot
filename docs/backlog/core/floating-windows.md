---
title: "Floating windows: dialogs and chosen apps float above the scrolling layout (auto for dialogs, window rules, toggle)"
status: "open"
area: "core"
priority: "high"
blocked: "queued after the current work (corner PR #240, wayland-rs fork repin) — user request 2026-09-24"
---

# Floating windows

User request, 2026-09-24: "we should investigate and implement a way to allow
specified windows to float. Like some other tiling ones do for things like
pop up confirmations or settings pop ups." Serves **daily-drive** first (a
file-save dialog or a settings window squeezed into a full-height column is
the most common "this compositor feels wrong" moment) and **computer use**
(an agent expects a confirmation dialog on top and centred, not a new column
off-screen).

## What is wrong today

Every `xdg_toplevel` is an independent column entry, dialogs included
(`resolved/wlr-foreign-toplevel-management-done.md`); `scoot-core` has no
floating concept. PR #240 made a short dialog at least draw and ring
correctly in its slot, but it still takes a column and scrolls the strip.

## What to decide (design first; the prior-art survey is part of the work)

- **Prior art to survey** (behaviour only — niri is GPL-3.0, OmniWM
  GPL-2.0: read for design, copy no code; sway/Hyprland similarly for
  behaviour): niri's floating layer + `window-rule { open-floating true }`
  and its default floating of dialogs; sway `for_window [...] floating
  enable` and its dialog heuristics; Hyprland `windowrule = float, ...`.
- **Which windows float automatically**, in order of signal strength:
  - `xdg-dialog-v1` (`xdg_wm_dialog_v1.get_xdg_dialog`, modal hint) —
    the standard for this; check Smithay's support at the pinned fork
    (`scoot-sh/smithay` `43f50eb`) and follow `CLAUDE.md`'s "standard
    protocol first" rule;
  - an `xdg_toplevel` with a parent (`set_parent`) — transient windows;
  - fixed size (min == max size set) — non-resizable dialogs;
  - decide whether the heuristics are on by default and configurable.
- **Window rules** in config (`[[window_rule]]` matching `app_id` /
  `title`, regex or glob, with `float = true` and perhaps an initial size /
  position) — the "specified windows" the user asked for. Reloadable
  (`scootctl reload`, see config-reload work).
- **Toggle**: an IPC action (`scootctl action toggle-floating`, and by-id
  `set-floating ID on|off` like fullscreen's pair) and a default bind.
- **Layout semantics in `scoot-core`** (platform-independent, fuzzed;
  `CLAUDE.md` says touch only for a real reason — this is one): a
  per-workspace floating set above the strip; position (centred on its
  parent / output, clamped to the usable area), size (client's preferred,
  clamped), stacking order and raise-on-focus, focus movement between
  floating and tiled (a keybind to switch), what `focus-column` does with
  a floating window focused, workspace moves, multi-output, fullscreen
  interplay (#223), what un-floating does (insert as a column where?).
- **Move/resize**: pointer move (Mod+drag) and resize for floating windows,
  or IPC-only first; `xdg_toplevel.move`/`resize` requests from CSD
  clients.
- **Knock-ons**: no tiled states for floating windows (PR #240 sends them
  to layout windows only — floating must not get them, so foot etc.
  cell-round again, which is correct), rounded clip + ring around floating
  windows, output clipping (#224), hit-testing and z-order in
  `window_under`, IPC `windows` (a `floating` field; `rect` is the real
  position), foreign-toplevel protocols, screenshots/primary-direct.

## Evidence expected

Core property/fuzz tests for the floating set; harness tests for auto-float
(dialog protocol, parent, fixed size), rules, toggle, focus and stacking;
live runs with real dialogs (GTK file chooser via `zenity --file-selection`,
a GTK settings/about dialog, `foot` via rule) with screenshots; docs for the
config syntax, binds and IPC actions (README gets a short user-facing line —
it is for users and prospective users).
