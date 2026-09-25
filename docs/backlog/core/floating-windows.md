---
title: "Floating windows: dialogs and chosen apps float above the scrolling layout (auto for dialogs, window rules, toggle)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Floating windows

**Status (2026-09-25): PR 1 of 2 landed (PR #TBD); PR 2 remains.** PR 1
is the floating layer, map-time auto-float, window rules, the toggle and
the focus switch, and IPC. **PR 2 is pointer move/resize**: Super+drag,
and the client's own `xdg_toplevel.move`/`resize` (a CSD titlebar drag or
resize edge), plus whatever repositioning surface comes with it. Until
then a floating window sits where scoot centred it; there is no IPC move
either (the ticket made one optional, and it would pre-empt PR 2's
position model). The ticket stays open for PR 2; the original entry is
below the design record.

## PR 1 design record

### Prior art (behaviour only; no code or config read or copied)

- **niri** (GPL-3.0; its wiki, `Floating-Windows`): a floating layout per
  workspace/monitor, always above the tiled one; windows with a parent or a
  fixed size float automatically; an `open-floating true|false` window rule
  forces it or disables the heuristic; `switch-focus-between-floating-and-tiling`
  moves focus; with a floating window focused the directional binds act on
  the floating layout; `toggle-window-floating`; move by IPC
  (`move-floating-window`) and by dragging.
- **sway**: `for_window [criteria] floating enable|disable`, criteria
  matched by regex; transient windows (a parent) and fixed-size windows
  (min == max) float by default; `floating toggle` on `Mod+Shift+space`,
  `focus mode_toggle` on `Mod+space`; floating windows are centred.
- **Hyprland**: `windowrule = float, <class/title regex>`; dialogs float by
  default.

All three agree on the heuristics (parent, fixed size) and on rules as the
override in both directions; scoot adds the one standard signal they
predate or use too (`xdg-dialog-v1`), takes sway's keys (the most widely
known for exactly these two actions), and differs on matching (globs, below).

### Decisions

- **Core state** (`scoot-core/src/world/floating.rs`, `tree.rs`): each
  workspace has a floating stack (bottom first; the most recently focused on
  top) and a flag for whether its focus is on that layer or its strip; each
  floating window carries its centre (relative to its output's area
  origin, so a bar appearing does not move it), the size the core asks
  for (usually none), and the width preset it had as a column. The strip is
  computed as if the layer did not exist; the randomized invariant test
  checks every tiled rect is unchanged across floating-only steps.
- **Placement**: centred once, when it floats, on its parent's visible part
  when the parent is visible on the same output and workspace, else on the
  usable area; then kept as a centre point, so a window that resizes itself
  grows around it. Always inside the usable area (bars excluded); re-centred
  when carried to another output. `arrange` does no parent lookup.
- **Size**: the client's choice (configure `0x0`), placed at what it drew
  (`FrameObserved.actual`); a rule's `size`, or a window that drew larger
  than the usable area, makes the core ask for a clamped size from then on
  (sticky, so asking to fit does not oscillate). Invisible until it has
  drawn, unless it was asked for a size.
- **Focus**: `toggle-floating-focus` (`Super+Space`) switches layer. With a
  floating window focused: `focus-column left|right` returns to the strip's
  focused column without stepping (floating windows are all centred in
  PR 1, so geometric left/right has nothing to go on -- revisit in PR 2
  once they can be moved); `focus-window up|down` cycles the stack (down
  raises the bottom-most, up sends the top to the bottom); the strip-only
  actions do nothing. A focused floating window closing hands focus to its
  parent on the same workspace (decided before the removal, so a dialog
  alone on an inactive workspace cannot refocus the workspace that slides
  into its index; and not to a parent stacked behind a fullscreen sibling,
  which would end that fullscreen -- both found in review, pinned by core
  tests), else the next floating window, else the strip.
- **Floating and un-floating**: floating takes the window out of its
  column; strip focus moves to the column on its left, and a window that
  floats before it ever drew (the map-time case) also puts the strip's
  scroll back, so the strip is exactly as it was. Un-floating inserts the
  window as a column right of the strip's focused column (the likely answer,
  and the one that makes float-then-unfloat restore the column order) at
  its old width.
- **Fullscreen**: a floating window can go fullscreen; it covers while it is
  the focused window and hides while focus is elsewhere; leaving returns it
  to floating (`0x0`, no tiled state). A covering tiled fullscreen window
  hides the floating layer while the strip has focus; a floating window
  taking focus (a dialog the fullscreen app opened) shows above it, and
  nothing counts as covering meanwhile (the `top` layer shows, direct
  scanout pauses) -- a dialog behind a fullscreen app looks like a hang.
  Floating or un-floating a fullscreen window ends the fullscreen first.
- **Auto-float, decided once at the first commit** (`scoot/src/compositor/floating.rs`):
  `xdg-dialog-v1` is implemented by the pinned Smithay fork (`dialog.rs`),
  so it is advertised (no fork change); then a parent; then a fixed size;
  all behind `[floating] auto` (default on). A later hint, parent or title
  change re-decides nothing; a re-map keeps the state. The window has
  already been sent a tiled configure at creation (`add_window`), so the
  decision adds a second configure in the same flush; on the wire every
  client measured drew its first frame at its own size.
- **Rules** (`window_rules.rs`): `[[window_rule]]` with `match_app_id` /
  `match_title` **globs** (`*`, `?`, whole string, case-sensitive) rather
  than regexes: the common rules are exact app ids (a regex needs anchors,
  and `foot` unanchored matches `footclient`) and title fragments; no new
  dependency and a bounded matcher; nothing needs captures. `float` and an
  optional `size`; later matching rules override earlier ones per field;
  a rule needs a matcher and must set something, else it is skipped with a
  warning (and refused by name on reload). Reloadable; map-time only.
- **Wire**: floating windows get no tiled states (the configure says so in
  the same configure as the size); rings are drawn directly under their own
  window so they show over the strip; pointer focus is re-derived when what
  floats on screen changes; IPC `windows` gains `floating`; actions
  `toggle-floating`, `set-floating ID on|off`, `toggle-floating-focus`
  (additive; no `PROTOCOL_VERSION` bump). Popups use PR #225's target
  unchanged (the window's output usable area); output clip (#224) and
  captures unchanged; primary-direct needs nothing (a floating fullscreen
  window covers like any other, and nothing covers while a floating window
  above a fullscreen one has focus).
- **X11 later**: the core takes `WindowInfo::parent` and
  `Event::FloatingRequested` from any platform, so XWayland Phase 2 maps
  `WM_TRANSIENT_FOR` and window types onto the same two inputs.

### Evidence

In the PR description: core property tests, 22 harness tests
(`floating/tests/`), the live dev VM run (`~/evidence/float/live/`), and
benchmarks.

---

The original entry follows.

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
